use super::*;
use crate::video_queue::field;

const RATIOS: &[&str] = &["1:1","2:3","3:2","3:4","4:3","9:16","16:9","21:9"];
const ROLES: &[&str] = &["identity","storyboard","scene","prop","reference"];

fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    value.as_object().filter(|value| value.keys().all(|key| keys.contains(&key.as_str())))
        .ok_or_else(|| invalid("工作流配置有未知字段或不是对象"))
}

fn slots(config: &Value) -> &[Value] {
    config["slots"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn max_asset_id(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| !id.trim().is_empty() && id.encode_utf16().count() <= 160)
}

fn validate_references(value: &Value) -> Result<()> {
    object(value,&["mode","slots","neutralAssetId","boardInFrameSlot"])?;
    if value["mode"] != "ordered-optional" || !matches!(value["boardInFrameSlot"].as_str(),None|Some("block")) ||
        value.get("boardInFrameSlot").is_some_and(|item| item != "block") {
        return Err(invalid("referenceConfig 需用 ordered-optional，首尾帧保持拦截看板"));
    }
    let items = value["slots"].as_array().filter(|items| items.len() <= 10).ok_or_else(|| invalid("参考槽须为数组，最多 10 槽"))?;
    let mut unique = std::collections::HashSet::new();
    for (index, slot) in items.iter().enumerate() {
        object(slot,&["index","nodeId","field","role","required","fallback"])?;
        let node = max_asset_id(&slot["nodeId"]).ok_or_else(|| invalid("参考槽缺少 nodeId"))?;
        if slot["index"].as_u64() != Some(index as u64+1) || !unique.insert(node) ||
            !slot["role"].as_str().is_some_and(|role| ROLES.contains(&role)) ||
            slot.get("field").is_some_and(|item| item != "image") {
            return Err(invalid("参考槽须从 1 按顺序声明，节点不能重复，并须填写 role；目标字段为 image"));
        }
        match slot["required"].as_bool() {
            Some(true) if slot.get("fallback").is_none() => {}
            Some(false) if slot["fallback"] == "neutral" && max_asset_id(&value["neutralAssetId"]).is_some() => {}
            _ => return Err(invalid("必填参考槽须保留真实素材；可选槽须声明 fallback: neutral 及项目中性图 ID")),
        }
    }
    if value.get("neutralAssetId").is_some() && max_asset_id(&value["neutralAssetId"]).is_none() {
        return Err(invalid("neutralAssetId 须为项目图片素材 ID"));
    }
    Ok(())
}

fn parse_dimensions(raw: &str) -> Option<(u64,u64)> {
    let (w,h) = raw.split_once('x')?;
    let (w,h) = (w.parse().ok()?,h.parse().ok()?);
    ((64..=8192).contains(&w) && (64..=8192).contains(&h)).then_some((w,h))
}

fn validate_output(value: &Value, kind: &str) -> Result<()> {
    object(value,&["aspectRatios","defaultAspectRatio","resolutionByRatio","durationNode","durationField","durationRange"])?;
    let ratios = value.get("aspectRatios").map(|value| value.as_array().filter(|items| !items.is_empty() && items.len() <= RATIOS.len())
        .ok_or_else(|| invalid("aspectRatios 须为非空画幅数组"))).transpose()?;
    let valid_aspect = |ratio: &str| RATIOS.contains(&ratio) && ratios.is_none_or(|ratios| ratios.iter().any(|item| item == ratio));
    if ratios.is_some_and(|ratios| {
        let mut seen = std::collections::HashSet::new();
        ratios.iter().any(|item| !item.as_str().is_some_and(|ratio| RATIOS.contains(&ratio) && seen.insert(ratio)))
    }) { return Err(invalid("画幅不受支持或重复")); }
    if value.get("defaultAspectRatio").is_some_and(|item| !item.as_str().is_some_and(valid_aspect)) {
        return Err(invalid("defaultAspectRatio 须在可选画幅内"));
    }
    if let Some(resolutions) = value.get("resolutionByRatio") {
        for (ratio, size) in object(resolutions,RATIOS)? {
            if !valid_aspect(ratio) || size.as_str().and_then(parse_dimensions).is_none() {
                return Err(invalid("resolutionByRatio 须按支持的画幅填写宽x高，像素范围 64 至 8192"));
            }
        }
    }
    if value.get("durationNode").is_some() || value.get("durationField").is_some() || value.get("durationRange").is_some() {
        if kind != "video" || value.get("durationNode").is_some_and(|item| max_asset_id(item).is_none()) ||
            value.get("durationField").is_some_and(|item| max_asset_id(item).is_none() || value.get("durationNode").is_none()) {
            return Err(invalid("durationNode、durationField 和 durationRange 只适用于视频；指定字段时须同时指定节点"));
        }
        if let Some(range) = value.get("durationRange") {
            let valid = range.as_array().filter(|items| items.len()==2).and_then(|items| Some((items[0].as_f64()?,items[1].as_f64()?)))
                .is_some_and(|(min,max)| min>0.0 && min<=max && max<=120.0);
            if !valid { return Err(invalid("durationRange 须为 0 至 120 秒内的 [最短,最长]")); }
        }
    }
    Ok(())
}

pub(super) fn update(profile: &mut Value, input: &Value, kind: &str) -> Result<()> {
    for key in ["referenceConfig","outputConfig"] {
        if let Some(patch) = input.get(key) {
            if patch.is_null() {
                if let Some(adapter) = profile["adapter"].as_object_mut() { adapter.remove(key); }
                continue;
            }
            let patch = patch.as_object().ok_or_else(|| invalid("工作流能力配置须为对象或 null"))?;
            let mut config = profile["adapter"][key].as_object().cloned().unwrap_or_default();
            for (field,value) in patch {
                if field == "resolutionByRatio" {
                    let update = value.as_object().ok_or_else(|| invalid("resolutionByRatio 须为对象"))?;
                    let mut sizes = config.get(field).and_then(Value::as_object).cloned().unwrap_or_default();
                    for (ratio,size) in update {
                        if size.is_null() { sizes.remove(ratio); }
                        else { sizes.insert(ratio.clone(),size.clone()); }
                    }
                    config.insert(field.clone(),json!(sizes));
                } else { config.insert(field.clone(),value.clone()); }
            }
            profile["adapter"][key] = json!(config);
        }
    }
    let adapter = &profile["adapter"];
    if adapter.get("referenceConfig").is_some() || adapter.get("outputConfig").is_some() {
        if !matches!(kind,"image"|"video") || adapter["type"] != "comfyui-prompt" {
            return Err(invalid("工作流能力配置仅适用于原生 ComfyUI API 生图或视频"));
        }
        if let Some(value) = adapter.get("referenceConfig") { validate_references(value)?; }
        if let Some(value) = adapter.get("outputConfig") { validate_output(value,kind)?; }
    }
    Ok(())
}

pub(super) fn explicit_bindings(profile: &Value) -> Vec<Value> {
    slots(&profile["adapter"]["referenceConfig"]).iter()
        .map(|slot| json!({"nodeId":slot["nodeId"],"field":"image","role":"image","mediaMode":"upload","source":"referenceConfig"})).collect()
}

pub(super) fn reference_slots(profile: &Value, bindings: &[&Value]) -> Result<Option<Vec<Value>>> {
    let Some(config) = profile["adapter"].get("referenceConfig") else { return Ok(None); };
    validate_references(config)?;
    let declared = slots(config);
    if declared.len() != bindings.len() || declared.iter().any(|slot| {
        !bindings.iter().any(|binding| binding["nodeId"] == slot["nodeId"] && binding["field"] == "image")
    }) { return Err(invalid("referenceConfig 须为每个普通图片输入声明用途，不可指向首尾帧、覆盖其他字段或遗漏输入")); }
    Ok(Some(declared.to_vec()))
}

pub(super) struct References {
    pub assignments: Vec<Value>,
    pub ids: Vec<String>,
    pub actual_ids: Vec<String>,
}

pub(super) fn assign_references(profile: &Value, bindings: &[&Value], input: &Value, ordered: &[Value]) -> Result<Option<References>> {
    let Some(slots) = reference_slots(profile,bindings)? else {
        if input.get("referenceAssignments").is_some() { return Err(invalid("按槽位提交需在模型配置中先声明 referenceConfig")); }
        return Ok(None);
    };
    let mut provided = std::collections::BTreeMap::new();
    if let Some(assignments) = input.get("referenceAssignments") {
        if input.get("referenceAssetIds").is_some() { return Err(invalid("referenceAssignments 和 referenceAssetIds 只能填写一个")); }
        let items = assignments.as_array().filter(|items| items.len() <= slots.len()).ok_or_else(|| invalid("参考槽赋值数量超过已声明的槽位"))?;
        for item in items {
            object(item,&["index","assetId"])?;
            let index = item["index"].as_u64().filter(|i| (1..=slots.len() as u64).contains(i))
                .ok_or_else(|| invalid("参考槽序号超出范围"))?;
            let id = max_asset_id(&item["assetId"]).ok_or_else(|| invalid("参考槽素材 ID 无效"))?;
            if provided.insert(index,id).is_some() { return Err(invalid("同一个参考槽不能重复赋值")); }
        }
    } else {
        if ordered.len() > slots.len() { return Err(invalid("参考图超出已声明的槽位数")); }
        for (i,id) in ordered.iter().enumerate() {
            if id.is_null() { continue; }
            provided.insert(i as u64+1,max_asset_id(id).ok_or_else(|| invalid("参考图 ID 无效"))?);
        }
    }
    let config = &profile["adapter"]["referenceConfig"];
    let mut result = References { assignments: vec![],ids: vec![],actual_ids: vec![] };
    for slot in slots {
        let index = slot["index"].as_u64().unwrap_or_default();
        let role = field(&slot,"role")?;
        let (asset,fallback) = if let Some(asset) = provided.get(&index) {
            result.actual_ids.push((*asset).to_owned());
            (*asset,false)
        } else if slot["required"] == true {
            return Err(invalid(&format!("缺少第 {index} 槽（{role}）参考图"))
                .with_issues(vec![json!({"code":"missing_reference_slot","index":index,"role":role,"nodeId":slot["nodeId"]})]));
        } else { (field(config,"neutralAssetId")?,true) };
        result.ids.push(asset.to_owned());
        result.assignments.push(json!({"index":index,"nodeId":slot["nodeId"],"role":role,
            "required":slot["required"],"assetId":asset,"fallback":fallback}));
    }
    Ok(Some(result))
}

pub(super) fn verify_roles(project: &Value, references: &References) -> Result<()> {
    let groups = [
        ("identity","subjects","designSheetAssetId","designSheetHistoryIds"),
        ("scene","scenes","plateAssetId","plateHistoryIds"),
        ("storyboard","shots","boardAssetId","boardHistoryIds"),
        ("prop","props","designSheetAssetId","designSheetHistoryIds"),
    ];
    let mut issues = vec![];
    for assignment in &references.assignments {
        let asset = &assignment["assetId"];
        let role = assignment["role"].as_str().unwrap_or_default();
        let actual: Vec<_> = groups.iter().filter_map(|(role,list,selected,history)|
            project[*list].as_array().into_iter().flatten().any(|item|
                item[*selected] == *asset || item[*history].as_array().is_some_and(|items| items.contains(asset))
            ).then_some(*role)).collect();
        if actual.is_empty() { continue; }
        if assignment["fallback"] == true || (role != "reference" && !actual.contains(&role)) {
            issues.push(json!({"code":if assignment["fallback"] == true { "neutral_asset_has_role" } else { "reference_role_mismatch" },
                "index":assignment["index"],"assetId":asset,"role":role,"actualRoles":actual}));
        }
    }
    if issues.is_empty() { Ok(()) } else {
        Err(invalid("参考槽素材与配置用途不符，或补位图被标为正式素材。请核对 issues").with_issues(issues))
    }
}

pub(super) fn aspect<'a>(profile: &'a Value, fallback: Option<&'a str>, requested: Option<&'a Value>) -> Result<&'a str> {
    let config = &profile["adapter"]["outputConfig"];
    if requested.is_some() && !config.is_object() { return Err(invalid("指定本镜画幅前，请在模型设置保存 outputConfig")); }
    let chosen = if let Some(value) = requested {
        value.as_str().ok_or_else(|| invalid("aspectRatio 须为已声明画幅"))?
    } else { fallback.or_else(|| config["defaultAspectRatio"].as_str()).unwrap_or("16:9") };
    if !RATIOS.contains(&chosen) || config["aspectRatios"].as_array().is_some_and(|items| !items.iter().any(|item| item == chosen)) {
        return Err(invalid(&format!("工作流未声明画幅 {chosen}，请查看 outputConfig.aspectRatios")));
    }
    Ok(chosen)
}

pub(super) fn configured_size(profile: &Value, ratio: &str) -> Option<(u64,u64)> {
    profile["adapter"]["outputConfig"]["resolutionByRatio"][ratio].as_str().and_then(parse_dimensions)
}

pub(super) fn duration(profile: &Value, seconds: f64) -> Result<()> {
    let range = &profile["adapter"]["outputConfig"]["durationRange"];
    if let (Some(min),Some(max)) = (range[0].as_f64(),range[1].as_f64()) {
        if !(min..=max).contains(&seconds) { return Err(invalid(&format!("本镜 {seconds} 秒，超出工作流时长范围 {min} 至 {max} 秒"))); }
    }
    Ok(())
}

pub(super) fn length_binding(profile: &Value, graph: &Value, existing: &[Value]) -> Result<Option<Value>> {
    let config = &profile["adapter"]["outputConfig"];
    let Some(id) = config["durationNode"].as_str() else { return Ok(None); };
    let prior = existing.iter().find(|binding| binding["role"] == "length" && binding["nodeId"] == id);
    let chosen = config["durationField"].as_str().or_else(|| prior.and_then(|binding| binding["field"].as_str()));
    let inputs = &graph[id]["inputs"];
    let numeric = |key: &str| inputs[key].is_number();
    let candidates: Vec<_> = ["seconds","duration","length","value"].into_iter().filter(|key| numeric(key)).collect();
    let key = chosen.or_else(|| if candidates.len() == 1 { candidates.first().copied() } else { None })
        .filter(|key| numeric(key)).ok_or_else(|| invalid("时长节点没有唯一可写的秒数输入，请设置 durationField"))?;
    Ok(Some(json!({"role":"length","nodeId":id,"field":key,"durationUnit":"seconds"})))
}

pub(super) fn selector_value(aspect: &str) -> Result<&'static str> {
    Ok(match aspect {
        "1:1" => "1:1 (Square)", "2:3" => "2:3 (Portrait Photo)", "3:2" => "3:2 (Photo)",
        "3:4" => "3:4 (Portrait Standard)", "4:3" => "4:3 (Standard)",
        "9:16" => "9:16 (Portrait Widescreen)", "16:9" => "16:9 (Widescreen)", "21:9" => "21:9 (Ultrawide)",
        _ => return Err(invalid("画幅不受 ResolutionSelector 支持")),
    })
}

pub(super) fn selector_bindings(profile: &Value, graph: &Value) -> Result<Vec<Value>> {
    let mapped: Vec<_> = profile["adapter"]["inputMap"]["bindings"].as_array().into_iter().flatten()
        .filter(|binding| binding["role"] == "aspect_ratio").cloned().collect();
    if !mapped.is_empty() || !profile["adapter"]["outputConfig"].is_object() { return Ok(mapped); }
    let detected: Vec<_> = graph.as_object().into_iter().flatten().filter(|(_,node)|
        node["class_type"] == "ResolutionSelector" && node["inputs"]["aspect_ratio"].is_string()
    ).map(|(id,_)| json!({"nodeId":id,"field":"aspect_ratio","role":"aspect_ratio"})).collect();
    if detected.len() > 1 { return Err(invalid("存在多个 ResolutionSelector，请配置明确的 aspect_ratio 绑定")); }
    Ok(detected)
}

pub(super) fn apply_selectors(graph: &mut Value, bindings: &[Value], aspect: &str) -> Result<Vec<Value>> {
    let mut applied = Vec::new();
    for binding in bindings {
        let node = &graph[field(binding,"nodeId")?];
        let value = if node["class_type"] == "ResolutionSelector" && binding["field"] == "aspect_ratio" { selector_value(aspect)? } else { aspect };
        video_queue::bind(graph,binding,json!(value))?;
        applied.push(json!({"nodeId":binding["nodeId"],"field":binding["field"],"value":value}));
    }
    Ok(applied)
}

pub(super) fn apply_size(graph: &mut Value, profile: &Value, selectors: &[Value], ratio: &str) -> Result<Option<Value>> {
    let Some((width,height)) = configured_size(profile,ratio) else { return Ok(None); };
    let bindings = profile["adapter"]["inputMap"]["bindings"].as_array().map(Vec::as_slice).unwrap_or_default();
    let single = |role: &str| -> Result<Option<&Value>> {
        let mut matches = bindings.iter().filter(|binding| binding["role"] == role);
        let first = matches.next();
        if matches.next().is_some() { return Err(invalid("每个输出尺寸字段须只绑定一次")); }
        Ok(first)
    };
    if let (Some(w),Some(h)) = (single("width")?,single("height")?) {
        let target = |value: &Value| (value["nodeId"].clone(),value["field"].clone());
        if target(w) == target(h) || [w,h].iter().any(|binding|
            graph[binding["nodeId"].as_str().unwrap_or_default()]["inputs"][binding["field"].as_str().unwrap_or_default()].is_number() == false
            || selectors.iter().any(|selector| target(selector) == target(binding))) {
            return Err(invalid("宽高须映射到互不相同的数字输入，并独立于画幅选择器"));
        }
        video_queue::bind(graph,w,json!(width))?;
        video_queue::bind(graph,h,json!(height))?;
        return Ok(Some(json!({"width":width,"height":height,"mode":"mapped-pixels"})));
    }
    if selectors.len() == 1 {
        let node = field(&selectors[0],"nodeId")?;
        if graph[node]["class_type"] == "ResolutionSelector" && graph[node]["inputs"]["megapixels"].is_number() {
            let mp = width as f64 * height as f64 / 1_000_000.0;
            video_queue::bind(graph,&json!({"nodeId":node,"field":"megapixels"}),json!(mp))?;
            return Ok(Some(json!({"width":width,"height":height,"mode":"selector-target","megapixels":mp})));
        }
    }
    Err(invalid("已配置画幅分辨率，但缺少宽高映射或可用的 ResolutionSelector"))
}
