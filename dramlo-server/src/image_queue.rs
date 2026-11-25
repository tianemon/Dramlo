use super::*;
use serde::Serialize;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) const METHODS: &[&str] = &[
    "studio_prepare_image", "studio_confirm_image", "studio_image_status", "studio_current_image_job", "studio_resume_image",
    "studio_attach_image_asset", "studio_import_image_asset", "studio_describe_image_inputs", "studio_discard_image_plan",
    "studio_resolve_unknown_image", "studio_pending_jobs",
];

type Lock = Option<(String, String)>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    project_id: String,
    request_id: String,
    revision: u64,
    target_type: String,
    target_id: String,
    target_snapshot: Value,
    profile_id: String,
    profile_stamp: u64,
    prompt: String,
    reference_asset_ids: Vec<String>,
    #[serde(default)]
    reference_assignments: Vec<Value>,
    #[serde(default)]
    reference_facts: Value,
    #[serde(default)]
    workload: Value,
    graph: Value,
    bindings: Vec<Value>,
    output_node_id: Option<String>,
    status: String,
    prompt_id: Option<String>,
    asset_id: Option<String>,
    selected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    main_image_synced: Option<bool>,
    error: Option<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    aspect: Option<Value>,
    #[serde(default)]
    prompt_layout: Option<Value>,
    #[serde(default)]
    prompt_source: Option<String>,
    #[serde(default)]
    openai_size: Option<String>,
    #[serde(default)]
    approval_inputs: Option<Value>,
    #[serde(default)]
    provider_error: Option<Value>,
    #[serde(default)]
    unknown_resolution: Option<Value>,
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| invalid("缺少生图任务字段"))
}

fn target_slot(kind: &str) -> Result<(&'static str, &'static str, &'static str, &'static str)> {
    match kind {
        "scene" => Ok(("scenes", "plateAssetId", "plateHistoryIds", "platePrompt")),
        "shot-image" => Ok(("shots", "imageAssetId", "imageHistoryIds", "imagePrompt")),
        "subject" => Ok(("subjects", "designSheetAssetId", "designSheetHistoryIds", "sheetPrompt")),
        "prop" => Ok(("props", "designSheetAssetId", "designSheetHistoryIds", "sheetPrompt")),
        "shot-board" => Ok(("shots", "boardAssetId", "boardHistoryIds", "boardPrompt")),
        "shot-first" => Ok(("shots", "firstFrameAssetId", "", "firstFramePrompt")),
        "shot-last" => Ok(("shots", "lastFrameAssetId", "", "lastFramePrompt")),
        // 画布自由节点：产物写在 node.data 里，不走顶层数组。
        "canvas-node" => Ok(("canvas", "outputAssetId", "imageHistoryIds", "prompt")),
        _ => Err(invalid("不支持的生图或素材槽位")),
    }
}

fn entity<'a>(project: &'a Value, kind: &str, id: &str) -> Result<&'a Value> {
    if kind == "canvas-node" {
        let node = project["canvas"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["id"] == id)
            .ok_or_else(|| invalid("找不到画布节点"))?;
        if !matches!(node["type"].as_str(), Some("imageGen") | Some("compose")) {
            return Err(invalid(
                "画布生图只支持图片节点和合成节点，视频节点请走 studio_prepare_video",
            ));
        }
        return Ok(&node["data"]);
    }
    let (group, _, _, _) = target_slot(kind)?;
    project[group].as_array().into_iter().flatten().find(|item| item["id"] == id)
        .ok_or_else(|| invalid("找不到生图目标"))
}

/// 画布节点没传参考图时，自动取上游节点的产物（最多三张）。
fn upstream_reference_ids(project: &Value, node_id: &str) -> Vec<Value> {
    let edges = project["canvas"]["edges"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let nodes = project["canvas"]["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    edges
        .iter()
        .filter(|edge| edge["target"].as_str() == Some(node_id))
        .filter_map(|edge| edge["source"].as_str())
        .filter_map(|source| nodes.iter().find(|node| node["id"].as_str() == Some(source)))
        .filter_map(|node| {
            node["data"]["outputAssetId"]
                .as_str()
                .filter(|asset| !asset.is_empty())
                .map(|asset| json!(asset))
        })
        .take(3)
        .collect()
}

fn profile(app: &App, id: Option<&str>) -> Result<Value> {
    let record: Value = serde_json::from_slice(&fs::read(kv_path(app, "settings")?)?).map_err(|_| invalid("图像设置无效"))?;
    let settings: Value = serde_json::from_str(field(&record, "value")?).map_err(|_| invalid("图像设置无效"))?;
    let explicit = id.is_some();
    let id = id.or_else(|| settings["activeImageId"].as_str()).ok_or_else(|| invalid("请选择图像模型配置"))?;
    let image = settings["images"].as_array().into_iter().flatten().find(|entry| entry["id"] == id)
        .ok_or_else(|| invalid("找不到图像模型配置"))?;
    if !matches!(image["adapter"]["type"].as_str(), Some("comfyui-prompt" | "openai-images")) {
        return Err(invalid(if explicit { "该图像配置不支持服务端生图，请指定原生 ComfyUI API 或 OpenAI Images 配置" } else {
            "当前默认图像配置不支持服务端生图，请用 profileId 指定原生 ComfyUI API 或 OpenAI Images 配置"
        }));
    }
    endpoint_check(image)?;
    Ok(image.clone())
}

pub(super) fn endpoint_check(profile: &Value) -> Result<()> {
    match profile["adapter"]["type"].as_str() {
        Some("openai-images") => openai_images::endpoint(profile).map(|_| ()),
        _ => video_queue::endpoint(profile).map(|_| ()),
    }
}

fn profile_stamp(profile: &Value) -> u64 {
    if profile["adapter"]["type"] == "openai-images" {
        return video_queue::stamp(&json!([profile["baseUrl"],profile["adapter"],profile["model"]]));
    }
    video_queue::profile_stamp(profile)
}

fn job_path(app: &App, project: &str, request: &str) -> Result<PathBuf> {
    kv_path(app, &format!("image-job:{}", json!([project, request])))
}

fn latest_path(app: &App, project: &str, kind: &str, id: &str) -> Result<PathBuf> {
    kv_path(app,&format!("image-latest:{}",json!([project,kind,id])))
}

fn latest(app: &App, project: &str, kind: &str, id: &str) -> Result<Option<Job>> {
    match fs::read_to_string(latest_path(app,project,kind,id)?) {
        Ok(request) => Ok(Some(read(&job_path(app,project,&request)?)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn active_key(project: &str, request: &str) -> (String, String) {
    (project.into(), format!("image:{request}"))
}

fn normalized(job: &mut Job, active: &Lock) {
    if active.as_ref() != Some(&active_key(&job.project_id,&job.request_id)) {
        job.status = match job.status.as_str() {
            "submitting" => "unknown", "waiting" => "paused", "starting" => "prepared", other => other,
        }.into();
    }
}

fn save(app: &App, job: &Job) -> Result<()> {
    atomic_save(&job_path(app, &job.project_id, &job.request_id)?, &serde_json::to_vec(job)
        .map_err(|_| invalid("无法保存生图任务"))?)
}

fn read(path: &Path) -> Result<Job> {
    serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid("生图任务记录无效"))
}

fn response(job: &Job) -> Value {
    json!({ "projectId":job.project_id,"requestId":job.request_id,"updatedAt":job.revision,
        "targetType":job.target_type,"targetId":job.target_id,"profileId":job.profile_id,"prompt":job.prompt,
        "referenceAssetIds":job.reference_asset_ids,"referenceAssignments":job.reference_assignments,
        "referenceFacts":job.reference_facts,"workload":job.workload,"outputNodeId":job.output_node_id,"status":job.status,
        "promptId":job.prompt_id,"assetId":job.asset_id,"selected":job.selected,"mainImageSynced":job.main_image_synced,"error":job.error,
        "warnings":job.warnings,"aspect":job.aspect,"promptLayout":job.prompt_layout,"promptSource":job.prompt_source,
        "providerError":job.provider_error,"unknownResolution":job.unknown_resolution,
        "nextAction":match job.status.as_str(){
            "discarded" if job.unknown_resolution.is_some() => Some(json!({"code":"new_request_id","message":"已在核对后关闭原请求。重新生成时使用新请求 ID 预检。"})),
            "failed" => Some(json!({"code":"new_request_id","message":"此 requestId 已失败。处理原因后，用新 requestId 重新预检。重复使用原 ID 只会返回原结果。"})),
            "unknown" => Some(json!({"code":"check_provider","message":"先核对供应商记录。确认原任务不再运行且没有可恢复结果时，可经用户确认关闭原记录。"})),
            _ => None
        } })
}

fn prompt_layout(input: &Value) -> Result<Option<Value>> {
    let Some(layout) = input.get("promptLayout") else { return Ok(None); };
    let fields = layout.as_object().ok_or_else(|| invalid("提示词版式资料须为对象"))?;
    if fields.is_empty() || fields.keys().any(|key| !matches!(key.as_str(),"title"|"aspectRatio"|"rows"|"columns")) ||
        layout.get("title").is_some_and(|title| title.as_str().is_none_or(|title| title.trim().is_empty() || title.chars().count()>160)) ||
        layout.get("aspectRatio").is_some_and(|value| value.as_str().is_none_or(|text| !matches!(text,"16:9"|"9:16"|"1:1"|"4:3"|"21:9"))) ||
        ["rows","columns"].into_iter().any(|key| layout.get(key).is_some_and(|value| value.as_u64().is_none_or(|n| !(1..=6).contains(&n)))) ||
        (layout.get("rows").is_some() != layout.get("columns").is_some()) {
        return Err(invalid("版式资料无效，行数与列数须同时提供"));
    }
    Ok(Some(layout.clone()))
}

pub(super) fn save_prompt(project: &mut Value, input: &Value) -> Result<Value> {
    let kind = field(input,"targetType")?;
    let id = field(input,"targetId")?;
    let prompt = field(input,"visualPrompt")?;
    if prompt.encode_utf16().count() > 12000 { return Err(invalid("生图正文不能超过 12000 字符")); }
    let (group,_,_,prompt_field) = target_slot(kind)?;
    let sync = input["syncScript"].as_bool().unwrap_or(false);
    if sync && kind != "shot-image" { return Err(invalid("syncScript 仅适用于镜头关键帧")); }
    let shot_id = if group == "shots" { Some(project_authoring::resolve_shot_id(project,id)?) } else { None };
    let id = shot_id.as_deref().unwrap_or(id);
    let layout_field = format!("{prompt_field}Layout");
    let target = project[group].as_array_mut().into_iter().flatten().find(|item|item["id"] == id)
        .ok_or_else(||studio_mcp::missing("找不到生图目标"))?;
    if input["promptLayout"].is_null() {
        if input.get("promptLayout").is_some() { target.as_object_mut().ok_or_else(||invalid("目标数据格式无效"))?.remove(&layout_field); }
    } else if let Some(layout) = prompt_layout(input)? {
        generation_review::validate_frame_layout(kind,Some(&layout))?;
        target[&layout_field] = layout;
    }
    target[prompt_field] = json!(prompt);
    if sync { target["visualPrompt"] = json!(prompt); }
    Ok(json!({"targetType":kind,"targetId":id,"promptField":prompt_field,"visualPrompt":target[prompt_field],
        "promptLayout":target[&layout_field]}))
}

fn ratio(value: &str) -> Option<(u32,u32)> {
    let (width, rest) = value.trim().split_once([':', '：'])?;
    let height = rest.trim_start().split(|ch: char| !ch.is_ascii_digit()).next()?;
    let pair: (u32,u32) = (width.trim().parse().ok()?, height.parse().ok()?);
    (pair.0 > 0 && pair.1 > 0).then_some(pair)
}

fn prompt_metadata(prompt: &str) -> Vec<&'static str> {
    let first = prompt.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
    let mut hits = Vec::new();
    for (left,right) in [('【','】'),('[',']')] {
        if first.strip_prefix(left).and_then(|body| body.split_once(right))
            .is_some_and(|(label,_)| ratio(label).is_some()) {
            hits.push("开头的画幅标签");
            break;
        }
    }
    if ["场景设定表","角色设定表","道具设定表","场景锚点图"].iter().any(|tag| first.contains(tag))
        && (first.contains('「') || first.contains('“') || first.contains("设定表")) {
        hits.push("设定表或锚点图标题");
    }
    if ["不够四项","四格按这个清单","按以下清单补齐"].iter().any(|marker| prompt.contains(marker)) {
        hits.push("分格补齐指令");
    }
    hits
}

fn aspect_diagnosis(project: &Value, graph: &Value, warnings: &mut Vec<String>) -> Value {
    let expected = project["settings"]["aspectRatio"].as_str();
    let selectors: Vec<_> = graph.as_object().into_iter().flatten().filter_map(|(id,node)| {
        if node["class_type"] != "ResolutionSelector" { return None; }
        let setting = node["inputs"]["aspect_ratio"].as_str()?;
        let (width,height) = ratio(setting)?;
        Some((id,setting,width,height))
    }).collect();
    let settings: Vec<_> = selectors.iter().map(|(id,setting,width,height)| {
        json!({"nodeId":id,"value":setting,"ratio":format!("{width}:{height}")})
    }).collect();
    let matches = expected.and_then(ratio).filter(|_| !selectors.is_empty()).map(|(width,height)| {
        selectors.iter().all(|(_,_,w,h)| u64::from(*w)*u64::from(height) == u64::from(*h)*u64::from(width))
    });
    if matches == Some(false) {
        let actual = selectors.iter().map(|(_,text,_,_)| *text).collect::<Vec<_>>().join("、");
        warnings.push(format!("项目画幅为 {}，工作流画幅设置为 {actual}。此次预检保留工作流设置，请核对后再确认。", expected.unwrap_or_default()));
    }
    json!({"projectAspectRatio":expected,"workflowSelectors":settings,"matchesProject":matches})
}

fn aspect_slots(profile: &Value) -> impl Iterator<Item = &Value> {
    profile["adapter"]["inputMap"]["bindings"].as_array().into_iter().flatten().filter(|binding|binding["role"] == "aspect_ratio")
}

fn workflow_graph(profile: &Value) -> Result<Value> {
    let mut graph: Value = serde_json::from_str(field(&profile["adapter"], "workflowTemplate")?)
        .map_err(|_| invalid("ComfyUI API 工作流无效"))?;
    if let Some(wrapper) = graph.get_mut("prompt") { graph = std::mem::take(wrapper); }
    graph.as_object().filter(|nodes| !nodes.is_empty() && nodes.len() <= 1500 &&
        nodes.values().all(|node| node["class_type"].is_string() && node["inputs"].is_object()))
        .ok_or_else(|| invalid("请选用 ComfyUI API 工作流"))?;
    Ok(graph)
}

fn prompt_candidates(graph: &Value) -> Vec<Value> {
    let mut template = std::collections::BTreeSet::new();
    for node in graph.as_object().into_iter().flat_map(|nodes| nodes.values()) {
        if node["class_type"] != "StringReplace" || node["inputs"]["find"] != "{input text}" { continue; }
        if let Some(link) = node["inputs"]["replace"].as_array().filter(|link| link.len() == 2 && link[1] == 0) {
            if let Some(id) = link[0].as_str().filter(|id| {
                matches!(graph[*id]["class_type"].as_str(), Some("PrimitiveString" | "PrimitiveStringMultiline"))
                    && graph[*id]["inputs"]["value"].is_string()
            }) { template.insert(id); }
        }
    }
    if !template.is_empty() {
        return template.into_iter().map(|id| json!({"role":"prompt","nodeId":id,"field":"value","source":"template-placeholder"})).collect();
    }
    let mut candidates: Vec<_> = video_queue::inferred_bindings(graph).into_iter().filter(|binding| binding["role"] == "prompt").collect();
    let text: Vec<_> = graph.as_object().into_iter().flatten()
        .filter(|(_, node)| node["class_type"] == "CLIPTextEncode" && node["inputs"]["text"].is_string()).collect();
    for (id, node) in &text {
        let title = node["_meta"]["title"].as_str().unwrap_or_default().to_ascii_lowercase();
        if (text.len() == 1 && candidates.is_empty()) || title.contains("positive") || title.contains("正向") {
            candidates.push(json!({"role":"prompt","nodeId":id,"field":"text"}));
        }
    }
    candidates
}

fn reference_candidates(graph: &Value) -> Vec<Value> {
    let Some(nodes) = graph.as_object() else { return vec![]; };
    let connected: std::collections::HashSet<_> = nodes.values().flat_map(|node| {
        node["inputs"].as_object().into_iter().flatten().filter_map(|(_, input)| {
            let link = input.as_array().filter(|link| link.len() == 2 && link[1].is_u64())?;
            link[0].as_str()
        })
    }).collect();
    nodes.iter().filter(|(id,node)| {
        node["class_type"].as_str().is_some_and(|kind| kind.to_ascii_lowercase().contains("loadimage"))
            && node["inputs"]["image"].is_string() && connected.contains(id.as_str())
    }).map(|(id,_)| json!({"role":"image","nodeId":id,"field":"image","source":"inferred"})).collect()
}

fn unbound_template_images(graph: &Value, candidates: &[Value], slots: &[Value]) -> Vec<Value> {
    candidates.iter().filter_map(|candidate| {
        let id = candidate["nodeId"].as_str()?;
        if slots.iter().any(|slot| slot["nodeId"] == id && slot["field"] == "image") { return None; }
        let filename = graph[id]["inputs"]["image"].as_str().filter(|name| !name.trim().is_empty())?;
        Some(json!({"nodeId":id,"field":"image","filename":filename}))
    }).collect()
}

fn reject_template_images(images: &[Value]) -> Result<()> {
    if images.is_empty() { return Ok(()); }
    let names = images.iter().filter_map(|image| image["filename"].as_str()).collect::<Vec<_>>().join("、");
    Err(invalid(&format!("生图工作流仍引用未绑定的固定图片：{names}。请配置参考图槽位并提供项目图片，或从工作流移除这些输入")))
}

fn reference_slots(graph: &Value, profile: &Value) -> Vec<Value> {
    let mut configured: Vec<_> = profile["adapter"]["inputMap"]["bindings"].as_array().into_iter().flatten()
        .filter(|binding| binding["role"] == "image").cloned().collect();
    for binding in comfy_config::explicit_bindings(profile) {
        if !configured.iter().any(|item| item["nodeId"] == binding["nodeId"] && item["field"] == "image") { configured.push(binding); }
    }
    if !configured.is_empty() { return configured; }
    let candidates = reference_candidates(graph);
    if candidates.len() == 1 { candidates } else { vec![] }
}

pub(super) fn inferred_mapping(graph: &Value) -> Vec<Value> {
    let candidates = prompt_candidates(graph);
    let mut bindings = if candidates.len() == 1 { candidates } else { vec![] };
    let references = reference_candidates(graph);
    if references.len() == 1 { bindings.extend(references); }
    bindings.extend(graph.as_object().into_iter().flatten().filter(|(_,node)|
        node["class_type"] == "ResolutionSelector" && node["inputs"]["aspect_ratio"].is_string())
        .map(|(node,_)|json!({"role":"aspect_ratio","nodeId":node,"field":"aspect_ratio"})));
    bindings
}

pub(super) fn mapping_issue(profile: &Value) -> Option<String> {
    workflow_inputs(profile).err().map(|failure| failure.1)
}

pub(super) fn missing_mapping(profile: &Value) -> Option<Vec<&'static str>> {
    let graph = workflow_graph(profile).ok()?;
    let configured = profile["adapter"]["inputMap"]["bindings"].as_array().into_iter().flatten();
    let configured_prompts = configured.filter(|binding| binding["role"] == "prompt").count();
    let mut missing = Vec::new();
    if configured_prompts == 0 && prompt_candidates(&graph).is_empty() { missing.push("prompt"); }
    if !unbound_template_images(&graph, &reference_candidates(&graph), &reference_slots(&graph, profile)).is_empty() {
        missing.push("image");
    }
    Some(missing)
}

fn workflow_inputs(profile: &Value) -> Result<(Value, Value, Vec<Value>)> {
    let graph = workflow_graph(profile)?;
    let configured = profile["adapter"]["inputMap"]["bindings"].as_array();
    let mut prompt_slots: Vec<Value> = configured.into_iter().flatten().filter(|b| b["role"] == "prompt").cloned().collect();
    if prompt_slots.is_empty() { prompt_slots = prompt_candidates(&graph); }
    if prompt_slots.len() != 1 { return Err(invalid("请配置唯一的正向提示词输入映射")); }
    let prompt = prompt_slots.pop().ok_or_else(|| invalid("请配置正向提示词输入映射"))?;
    let slots = reference_slots(&graph, profile);
    let candidates = reference_candidates(&graph);
    if slots.len() > 10 { return Err(invalid("参考图槽位不能超过 10 个")); }
    let declared = comfy_config::reference_slots(profile,&slots.iter().collect::<Vec<_>>())?;
    if declared.is_some() && slots.iter().any(|slot| !candidates.iter().any(|candidate| candidate["nodeId"] == slot["nodeId"])) {
        return Err(invalid("referenceConfig 指向未连接的图片节点"));
    }
    let mut used = std::collections::HashSet::new();
    let selectors = comfy_config::selector_bindings(profile,&graph)?;
    for binding in std::iter::once(&prompt).chain(&slots).chain(&selectors) {
        let node = field(binding, "nodeId")?;
        let key = field(binding, "field")?;
        if !used.insert((node,key)) { return Err(invalid("工作流输入映射重复")); }
        if !graph[node]["inputs"].get(key).is_some_and(|value| value.is_string()) {
            return Err(invalid("工作流文本或参考图映射已失效"));
        }
    }
    if slots.iter().any(|slot| slot["mediaMode"].as_str().is_some_and(|mode| mode != "upload")) {
        return Err(invalid("参考图映射须使用上传模式"));
    }
    reject_template_images(&unbound_template_images(&graph, &candidates, &slots))?;
    if slots.is_empty() && candidates.len() > 1 {
        return Err(invalid("发现多个参考图候选，请根据诊断明确槽位及顺序"));
    }
    Ok((graph, prompt, slots))
}

fn describe_inputs(app: &App, input: &Value) -> Result<Value> {
    let profile = profile(app, input["profileId"].as_str())?;
    describe_profile_inputs(&profile)
}

pub(super) fn describe_profile_inputs(profile: &Value) -> Result<Value> {
    if profile["adapter"]["type"] == "openai-images" {
        let ready = openai_images::ready(profile);
        return Ok(json!({"profileId":profile["id"],"profileName":profile["name"],"adapterType":"openai-images",
            "referenceCount":null,"minReferences":0,"maxReferences":openai_images::MAX_REFERENCES,
            "referenceSlots":[],"referenceMode":"ordered-optional","ready":ready.is_ok(),
            "issue":ready.err().map(|failure|failure.1),"templateImages":[]}));
    }
    let graph = workflow_graph(profile)?;
    let diagnosis = workflow_inputs(profile);
    let slots = reference_slots(&graph, profile);
    let candidates = reference_candidates(&graph);
    let template_images = unbound_template_images(&graph, &candidates, &slots);
    let declared = comfy_config::reference_slots(profile,&slots.iter().collect::<Vec<_>>()).ok().flatten();
    let mut items: Vec<_> = slots.into_iter().enumerate().map(|(index, slot)| {
        let node = slot["nodeId"].as_str().map(|id| &graph[id]);
        json!({"index":index,"nodeId":slot["nodeId"],"field":slot["field"],
            "mediaMode":slot["mediaMode"].as_str().unwrap_or("upload"),"source":slot["source"].as_str().unwrap_or("configured"),
            "label":node.and_then(|node| node["_meta"]["title"].as_str().or_else(|| node["class_type"].as_str()))})
    }).collect();
    let min = declared.as_ref().map_or(items.len(),|slots| slots.iter().filter(|slot| slot["required"] == true).count());
    if let Some(declared) = &declared {
        items = declared.iter().map(|slot| {
            let mut item = items.iter().find(|item| item["nodeId"] == slot["nodeId"]).unwrap().clone();
            for key in ["index","role","required","fallback"] {
                if let Some(value) = slot.get(key) { item[key] = value.clone(); }
            }
            item
        }).collect();
    }
    Ok(json!({"profileId":profile["id"],"profileName":profile["name"],
        "referenceCount":items.len(),"referenceSlots":items,"referenceMode":if declared.is_some() { "ordered-optional" } else { "fixed-ordered" },
        "minReferences":min,"maxReferences":items.len(),
        "ready":diagnosis.is_ok(),"issue":diagnosis.err().map(|failure|failure.1),
        "mappingSummary":profile["adapter"]["inputMap"]["summary"],
        "promptCandidates":prompt_candidates(&graph),"referenceCandidates":candidates,"templateImages":template_images,
        "aspectRatioBindings":aspect_slots(profile).collect::<Vec<_>>()}))
}

fn prepare(app: &App, input: &Value, project_id: &str, request: &str, pending: &[String]) -> Result<Job> {
    let (file, project) = studio_mcp::project_by_id(app, project_id)?;
    prepare_for(app,input,&file,&project,project_id,request,pending)
}

fn prepare_for(app: &App, input: &Value, file: &Path, project: &Value, project_id: &str, request: &str, pending: &[String]) -> Result<Job> {
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;
    let kind = field(input, "targetType")?;
    let requested_target = field(input, "targetId")?;
    let shot_id = kind.starts_with("shot-").then(|| project_authoring::resolve_shot_id(project,requested_target)).transpose()?;
    let target = shot_id.as_deref().unwrap_or(requested_target);
    let snapshot = entity(project, kind, target)?;
    let (_, _, _, prompt_field) = target_slot(kind)?;
    if input.get("visualPrompt").is_some() && input.get("prompt").is_some() {
        return Err(invalid("visualPrompt 和 prompt 只能填写一个"));
    }
    let prompt_layout = prompt_layout(input)?.or_else(|| snapshot.get(format!("{prompt_field}Layout")).cloned());
    generation_review::validate_frame_layout(kind,prompt_layout.as_ref())?;
    let (raw,source) = if let Some(prompt) = input.get("visualPrompt").or_else(|| input.get("prompt")) {
        (prompt,"request")
    } else if snapshot[prompt_field].as_str().is_some_and(|text|!text.trim().is_empty()) || kind != "shot-image" {
        (&snapshot[prompt_field],prompt_field)
    } else {
        (&snapshot["visualPrompt"],"visualPrompt")
    };
    let prompt = raw.as_str().map(str::trim)
        .filter(|text| !text.is_empty() && text.encode_utf16().count() <= 12000)
        .ok_or_else(|| invalid("请填写本次生图提示词，或先保存目标的生图提示词"))?;
    let profile = profile(app, input["profileId"].as_str())?;
    let output_node_id = input["outputNodeId"].as_str().map(str::to_owned);
    // 画布节点没显式传参考图时，自动吃上游节点的产物。
    let mut reference_values: Vec<Value> = input["referenceAssetIds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let auto_referenced = kind == "canvas-node" && input.get("referenceAssetIds").is_none();
    if auto_referenced {
        reference_values = upstream_reference_ids(project, target);
    }
    let ordered: &[Value] = &reference_values;
    let declared_shape = profile["adapter"]["referenceConfig"].is_object();
    let references = ordered.iter().filter(|id| !(declared_shape && id.is_null())).map(|id| {
        id.as_str().filter(|id| !id.is_empty()).map(str::to_owned).ok_or_else(|| invalid("参考图 ID 无效"))
    }).collect::<Result<Vec<_>>>()?;
    let mut warnings = Vec::new();
    if auto_referenced && !ordered.is_empty() {
        warnings.push(format!(
            "画布节点没有指定参考图，已自动取上游 {} 张",
            ordered.len()
        ));
    }
    let hits = prompt_metadata(prompt);
    if !hits.is_empty() {
        warnings.push(format!("提示词含{}，可能被渲染成画面内文字。请保留画面描述，把画幅设在工作流中。",hits.join("、")));
    }
    if profile["adapter"]["type"] == "openai-images" {
        if input.get("referenceAssignments").is_some() || input.get("aspectRatio").is_some() {
            return Err(invalid("OpenAI Images 按参考图数组和项目画幅执行"));
        }
        openai_images::ready(&profile)?;
        if output_node_id.is_some() { return Err(invalid("outputNodeId 只适用于 ComfyUI 工作流")); }
        if references.len() > openai_images::MAX_REFERENCES { return Err(invalid("OpenAI Images 最多支持 10 张项目参考图")); }
        if kind.starts_with("shot-") {
            warnings.extend(generation_review::review_cast(project,snapshot,prompt,
                (!references.iter().any(|id| pending.contains(id))).then_some(references.as_slice()),input.get("bindingReview"))?);
        }
        openai_images::check_references(&reference_paths(app,file,&references,pending)?)?;
        let project_ratio = project_aspect(project,true);
        let approval = approval_inputs(app,file,project,true,&references,pending)?;
        let openai_ratio = project_ratio.and_then(ratio);
        let (size,actual) = openai_images::size(field(&profile,"model")?,openai_ratio.unwrap_or((1,1)));
        if openai_ratio.is_some_and(|expected| expected != actual) {
            warnings.push(format!("当前模型使用最接近的输出尺寸 {size}，请在确认前核对画幅"));
        }
        if kind.starts_with("shot-") && references.is_empty()
            && snapshot["subjectIds"].as_array().is_some_and(|ids| !ids.is_empty()) {
            warnings.push("本镜绑定了人物，但本次没有传 referenceAssetIds，不会提交人物参考图".to_owned());
        }
        let workload = generation_review::workload("openai-images",1,references.len(),0.0,json!(size));
        let reference_facts = generation_review::reference_facts(project,snapshot,&references);
        let reference_assignments = references.iter().enumerate().map(|(index, id)|
            json!({"index":index,"assetId":id,"fallback":false})).collect();
        return Ok(Job { project_id: project_id.into(), request_id: request.into(), revision, target_type: kind.into(),
            target_id: target.into(), target_snapshot: snapshot.clone(), profile_id: field(&profile,"id")?.into(),
            profile_stamp: profile_stamp(&profile), prompt: prompt.into(), reference_asset_ids: references,
            reference_assignments, reference_facts, workload, graph: Value::Null, bindings: vec![], output_node_id, status: "prepared".into(), prompt_id: None,
            asset_id: None, selected: false, main_image_synced: None, error: None, warnings,
            aspect: Some(json!({"projectAspectRatio":project_ratio,"size":size,"matchesProject":openai_ratio.map(|expected| expected == actual)})),
            prompt_layout,prompt_source:Some(source.into()),openai_size:Some(size.into()),approval_inputs:Some(approval),provider_error:None,unknown_resolution:None });
    }
    let (mut graph, prompt_slot, mut image_slots) = workflow_inputs(&profile)?;
    let chosen = comfy_config::assign_references(&profile,&image_slots.iter().collect::<Vec<_>>(),input,ordered)?;
    let used_refs = chosen.as_ref().map_or(references.as_slice(),|items| items.actual_ids.as_slice());
    if kind.starts_with("shot-") {
        let refs = (!image_slots.is_empty() && !used_refs.iter().any(|id| pending.contains(id))).then_some(used_refs);
        warnings.extend(generation_review::review_cast(project,snapshot,prompt,refs,input.get("bindingReview"))?);
    }
    let reference_facts = generation_review::reference_facts(project,snapshot,used_refs);
    let reference_assignments = chosen.as_ref().map(|items|items.assignments.clone()).unwrap_or_else(||
        image_slots.iter().zip(&references).enumerate().map(|(index, (slot, id))|
            json!({"index":index,"nodeId":slot["nodeId"],"assetId":id,"fallback":false})).collect());
    let references = if let Some(items) = chosen {
        comfy_config::verify_roles(project,&items)?;
        if items.assignments.iter().any(|assignment| assignment["fallback"] == true) {
            let neutral = field(&profile["adapter"]["referenceConfig"],"neutralAssetId")?;
            video_queue::media_path(app,file,neutral,"image")
                .map_err(|_| invalid(&format!("中性图 {neutral} 不在当前项目中或无法读取")))?;
        }
        image_slots.sort_by_key(|slot| items.assignments.iter().position(|item| item["nodeId"] == slot["nodeId"]));
        items.ids
    } else { references };
    if output_node_id.as_ref().is_some_and(|id| graph.get(id).is_none()) { return Err(invalid("找不到指定输出节点")); }
    if references.len() != image_slots.len() || references.len() > 10 {
        return Err(invalid(&format!("此配置需要 {} 张参考图，收到 {} 张。用 studio_describe_image_inputs 核对槽位", image_slots.len(), references.len())));
    }
    video_queue::bind(&mut graph, &prompt_slot, json!(prompt))?;
    let approval = approval_inputs(app,file,project,false,&references,pending)?;
    for binding in &image_slots { video_queue::bind(&mut graph, binding, json!("studio-reference"))?; }
    let selectors = comfy_config::selector_bindings(&profile,&graph)?;
    let ratio = comfy_config::aspect(&profile,project["settings"]["aspectRatio"].as_str(),input.get("aspectRatio"))?;
    let size = comfy_config::apply_size(&mut graph,&profile,&selectors,ratio)?;
    let applied = comfy_config::apply_selectors(&mut graph,&selectors,ratio)?;
    if input.get("aspectRatio").is_some() && applied.is_empty() && size.is_none() {
        return Err(invalid("工作流缺少可写画幅或宽高映射"));
    }
    let mut aspect = aspect_diagnosis(project,&graph,&mut warnings);
    if profile["adapter"]["outputConfig"].is_object() {
        aspect["requestedAspectRatio"] = json!(ratio);
        aspect["configuredSize"] = json!(size);
    }
    if !applied.is_empty() { aspect["appliedBindings"] = json!(applied); }
    let workload = generation_review::workload("comfyui",1,references.len(),0.0,aspect["configuredSize"].clone());
    let aspect = Some(aspect);
    if profile["adapter"]["inputMap"]["summary"].as_str().is_some_and(|text| text.contains("未生效")) {
        warnings.push("此配置曾退回规则识别，请检查提示词和参考图映射".to_owned());
    }
    if kind.starts_with("shot-") && image_slots.is_empty()
        && snapshot["subjectIds"].as_array().is_some_and(|ids| !ids.is_empty()) {
        warnings.push("本镜绑定了人物，但当前工作流没有可用参考图槽，不会提交人物参考图".to_owned());
    }
    Ok(Job { project_id: project_id.into(), request_id: request.into(), revision, target_type: kind.into(),
        target_id: target.into(), target_snapshot: snapshot.clone(), profile_id: field(&profile,"id")?.into(),
        profile_stamp: profile_stamp(&profile), prompt: prompt.into(), reference_asset_ids: references,
        graph, reference_assignments, reference_facts, workload, bindings: image_slots, output_node_id, status: "prepared".into(), prompt_id: None,
        asset_id: None, selected: false, main_image_synced: None, error: None, warnings, aspect, prompt_layout,prompt_source:Some(source.into()),openai_size:None,approval_inputs:Some(approval),provider_error:None,unknown_resolution:None })
}

pub(super) fn inspect_plan(app: &App, input: &Value, file: &Path, project: &Value, pending: &[String]) -> Result<Value> {
    let job = prepare_for(app,input,file,project,field(input,"projectId")?,field(input,"requestId")?,pending)?;
    Ok(json!({"profileId":job.profile_id,"slots":job.bindings,"warnings":job.warnings,
        "referenceAssignments":job.reference_assignments,"referenceFacts":job.reference_facts,"workload":job.workload,
        "approvalStamp":video_queue::stamp(&json!([job.profile_stamp,job.prompt,job.aspect,job.output_node_id]))}))
}

pub(super) fn plan_reference_slots(app: &App, input: &Value) -> Vec<Value> {
    profile(app,input["profileId"].as_str()).ok()
        .filter(|profile| profile["adapter"]["type"] == "comfyui-prompt")
        .and_then(|profile| workflow_inputs(&profile).ok())
        .map_or_else(Vec::new,|(_,_,slots)|slots)
}

fn apply_asset(project: &mut Value, kind: &str, id: &str, asset: &str, selected: bool) -> Result<()> {
    if kind == "canvas-node" {
        let node = project["canvas"]["nodes"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .find(|item| item["id"] == id)
            .ok_or_else(|| invalid("找不到画布节点"))?;
        let old = node["data"]["outputAssetId"].as_str().map(str::to_owned);
        let mut values = node["data"]["imageHistoryIds"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        values.retain(|value| value != asset);
        let candidate = if selected {
            old.as_deref().filter(|value| *value != asset)
        } else {
            Some(asset)
        };
        if let Some(candidate) = candidate.filter(|value| !values.contains(&json!(value))) {
            values.insert(0, json!(candidate));
        }
        node["data"]["imageHistoryIds"] = json!(values);
        if selected {
            node["data"]["outputAssetId"] = json!(asset);
            node["status"] = json!("success");
        }
        return Ok(());
    }
    let (group, slot, history, _) = target_slot(kind)?;
    let item = project[group].as_array_mut().into_iter().flatten().find(|item| item["id"] == id)
        .ok_or_else(|| invalid("找不到素材绑定目标"))?;
    let old = item[slot].as_str().map(str::to_owned);
    if !history.is_empty() {
        let mut values = item[history].as_array().cloned().unwrap_or_default();
        values.retain(|value| value != asset);
        let candidate = if selected { old.as_deref().filter(|value| *value != asset) } else { Some(asset) };
        if let Some(candidate) = candidate.filter(|value| !values.contains(&json!(value))) { values.insert(0, json!(candidate)); }
        item[history] = json!(values);
    }
    if selected {
        item[slot] = json!(asset);
        let canvas_kind = match kind {
            "scene" => Some(("scene","sceneId")), "shot-image" => Some(("shotImage","shotId")),
            "shot-board" => Some(("shotBoard","shotId")), "subject" => Some(("subject","subjectId")),
            "prop" => Some(("prop","propId")), _ => None,
        };
        if let Some((node_type,key)) = canvas_kind {
            for node in project["canvas"]["nodes"].as_array_mut().into_iter().flatten() {
                if node["type"] == node_type && node["data"][key] == id { node["status"] = json!("success"); }
            }
        }
    }
    Ok(())
}

fn persist_project(file: &Path, project: &mut Value) -> Result<()> {
    project["updatedAt"] = json!(video_queue::now().max(project["updatedAt"].as_u64().unwrap_or_default().saturating_add(1)));
    atomic_save(file, &serde_json::to_vec_pretty(project).map_err(|_| invalid("无法保存项目"))?)
}

pub(super) fn image_format(bytes: &[u8]) -> Result<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Ok(("png","image/png")) }
    else if bytes.starts_with(b"\xff\xd8\xff") { Ok(("jpg","image/jpeg")) }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") { Ok(("webp","image/webp")) }
    else { Err(invalid("不是受支持的图片格式")) }
}

fn store_image(project_file: &Path, id: &str, bytes: &[u8], mut entry: Value) -> Result<bool> {
    let (extension, mime) = image_format(bytes)?;
    let folder = project_file.parent().ok_or_else(|| invalid("项目目录无效"))?;
    let filename = format!("{id}.{extension}");
    let relative = format!("assets/images/{filename}");
    let image = scoped_path(folder, &folder.join(&relative))?;
    let manifest = scoped_path(folder, &folder.join("assets/manifest.json"))?;
    let mut entries: Vec<Value> = match fs::read(&manifest) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("素材清单无效"))?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => vec![],
        Err(err) => return Err(err.into())
    };
    if let Some(existing) = entries.iter().find(|entry| entry["id"] == id) {
        if existing["fileRelPath"] != relative || fs::read(&image)? != bytes {
            return Err(invalid("同一请求 ID 已用于另一张图片"));
        }
        return Ok(false);
    }
    entry["id"] = json!(id);
    entry["kind"] = json!("image");
    entry["filename"] = json!(filename);
    entry["fileRelPath"] = json!(relative);
    entry["mimeType"] = json!(mime);
    entry["size"] = json!(bytes.len());
    entry["createdAt"] = json!(video_queue::now());
    entries.push(entry);
    let manifest_bytes = serde_json::to_vec_pretty(&entries).map_err(|_| invalid("无法保存素材清单"))?;
    atomic_save(&image, bytes)?;
    atomic_save(&manifest, &manifest_bytes)?;
    Ok(true)
}

fn archive(app: &App, job: &mut Job, bytes: &[u8]) -> Result<()> {
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let (file, mut project) = studio_mcp::project_by_id(app, &job.project_id)?;
    let id = format!("image_{}", job.request_id);
    let frame_role = (job.target_type == "shot-board" ||
        job.prompt_layout.as_ref().is_some_and(generation_review::is_grid)).then_some("board");
    store_image(&file, &id, bytes, json!({"source":"ai-generated","projectId":job.project_id,"frameRole":frame_role,
        "prompt":job.prompt,"imageGeneration":{"requestId":job.request_id,"profileId":job.profile_id,
        "targetType":job.target_type,"targetId":job.target_id,"promptId":job.prompt_id}}))?;
    let (_, slot, _, _) = target_slot(&job.target_type)?;
    if let Ok(current) = entity(&project, &job.target_type, &job.target_id) {
        let still_approved = check_approval(app,job,&file,&project).is_ok()
            && profile(app,Some(&job.profile_id)).is_ok_and(|profile| profile_stamp(&profile) == job.profile_stamp);
        job.selected = current[slot] == id || (current == &job.target_snapshot && still_approved);
        let sync_main = job.target_type == "shot-first" && job.selected && current[slot] != id &&
            (current["imageAssetId"].as_str().is_none_or(str::is_empty) || current["imageAssetId"] == current[slot]);
        if job.target_type == "shot-first" { job.main_image_synced = Some(sync_main); }
        if !job.selected { job.warnings.push("生成期间目标或输入已变化，图片已归档为候选。".into()); }
        apply_asset(&mut project, &job.target_type, &job.target_id, &id, job.selected)?;
        if sync_main { apply_asset(&mut project,"shot-image",&job.target_id,&id,true)?; }
    }
    persist_project(&file, &mut project)?;
    job.asset_id = Some(id);
    Ok(())
}

fn reference_paths(app: &App, file: &Path, ids: &[String], pending: &[String]) -> Result<Vec<PathBuf>> {
    ids.iter().filter(|id| !pending.contains(id)).map(|id| video_queue::media_path(app,file,id,"image")).collect()
}

fn project_aspect(project: &Value, openai: bool) -> Option<&str> {
    project["settings"]["aspectRatio"].as_str().or_else(|| {
        if openai { project["script"]["aspectRatio"].as_str() } else { None }
    })
}

fn approval_inputs(app: &App, file: &Path, project: &Value, openai: bool, ids: &[String], pending: &[String]) -> Result<Value> {
    let refs = reference_paths(app,file,ids,pending)?.into_iter().map(|path| {
        let meta = fs::metadata(&path)?;
        let modified = meta.modified()?.duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| invalid("参考图修改时间无效"))?;
        Ok(json!([path,meta.len(),modified.as_secs(),modified.subsec_nanos()]))
    }).collect::<Result<Vec<_>>>()?;
    Ok(json!({"aspectRatio":project_aspect(project,openai),"references":refs}))
}

fn check_approval(app: &App, job: &Job, file: &Path, project: &Value) -> Result<()> {
    let current = project["updatedAt"].as_u64();
    let Some(expected) = &job.approval_inputs else {
        // Older saved plans did not retain a snapshot of the generation inputs.
        return if current == Some(job.revision) { Ok(()) }
            else { Err(Failure::revision_conflict(current,Some(job.revision))) };
    };
    let target_matches = entity(project,&job.target_type,&job.target_id).is_ok_and(|target| target == &job.target_snapshot);
    let inputs_match = approval_inputs(app,file,project,job.openai_size.is_some(),&job.reference_asset_ids,&[])
        .is_ok_and(|inputs| &inputs == expected);
    if target_matches && inputs_match { Ok(()) }
    else { let mut conflict = Failure::revision_conflict(current,Some(job.revision)); conflict.1 = "本次生图的目标、画幅或参考图已变更，请重新预检".into(); Err(conflict) }
}

async fn generate_openai(app: &App, job: &mut Job, profile: &Value, size: &str) -> Result<()> {
    openai_images::ready(profile)?;
    let (file, project) = studio_mcp::project_by_id(app, &job.project_id)?;
    check_approval(app,job,&file,&project)?;
    let paths = reference_paths(app,&file,&job.reference_asset_ids,&[])?;
    let request = openai_images::request(profile,&job.prompt,size,&paths).await?;
    {
        let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
        let (file,current) = studio_mcp::project_by_id(app,&job.project_id)?;
        check_approval(app,job,&file,&current)?;
        let current_profile = self::profile(app,Some(&job.profile_id))?;
        if profile_stamp(&current_profile) != job.profile_stamp || current_profile["apiKey"] != profile["apiKey"] {
            return Err(invalid("图像接入已变化，请重新预检"));
        }
        job.status = "submitting".into();
        save(app,job)?;
    }
    let reply = openai_images::send(request).await?;
    let status = reply.status();
    if !status.is_success() {
        if status.is_client_error() && status != StatusCode::REQUEST_TIMEOUT {
            job.status = "failed".into();
        }
        let (diagnostic, message) = openai_images::diagnose_error(reply).await;
        job.provider_error = Some(diagnostic);
        return Err(invalid(&message));
    }
    let bytes = openai_images::read_image(reply).await?;
    archive(app,job,&bytes)
}

async fn generate(app: &App, job: &mut Job) -> Result<()> {
    let profile = profile(app, Some(&job.profile_id))?;
    if profile_stamp(&profile) != job.profile_stamp { return Err(invalid("图像接入已变化，请先核对原任务")); }
    if let Some(size) = job.openai_size.clone() {
        return generate_openai(app,job,&profile,&size).await;
    }
    if job.prompt_id.is_none() {
        if job.bindings.len() != job.reference_asset_ids.len() {
            return Err(invalid("生图任务的参考图绑定不完整，请重新预检"));
        }
        reject_template_images(&unbound_template_images(&job.graph, &reference_candidates(&job.graph), &job.bindings))?;
        let (file, project) = studio_mcp::project_by_id(app, &job.project_id)?;
        check_approval(app,job,&file,&project)?;
        let mut graph = job.graph.clone();
        for (index, (binding, id)) in job.bindings.iter().zip(&job.reference_asset_ids).enumerate() {
            let path = video_queue::media_path(app, &file, id, "image")?;
            let name = video_queue::upload(&profile, &path, &format!("{}-{index}", job.request_id)).await?;
            video_queue::bind(&mut graph, binding, json!(name))?;
        }
        {
            let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
            let (file,project) = studio_mcp::project_by_id(app,&job.project_id)?;
            check_approval(app,job,&file,&project)?;
            if profile_stamp(&self::profile(app,Some(&job.profile_id))?) != job.profile_stamp {
                return Err(invalid("图像接入已变化，请重新预检"));
            }
            job.status = "submitting".into();
            save(app, job)?;
        }
        let payload = json!({"prompt":graph,"client_id":job.request_id});
        let (status, _, body) = video_queue::http(&profile,"POST","/prompt","application/json",payload.to_string().into_bytes()).await?;
        if status == StatusCode::BAD_REQUEST {
            job.status = "failed".into();
            return Err(invalid("ComfyUI 拒绝生图工作流"));
        }
        if !status.is_success() { return Err(invalid("提交结果未知，请核对 ComfyUI 后再决定是否重做")); }
        job.prompt_id = Some(field(&video_queue::json_body(&body)?,"prompt_id")?.to_owned());
        job.status = "waiting".into();
        save(app, job)?;
    }
    let id = job.prompt_id.clone().ok_or_else(|| invalid("缺少远端任务 ID"))?;
    for _ in 0..450 {
        let route = format!("/history/{}", video_queue::escape(&id));
        let (status, _, body) = video_queue::http(&profile,"GET",&route,"application/json",Vec::new()).await?;
        if status.is_success() {
            let history = video_queue::json_body(&body)?;
            let entry = &history[&id];
            if entry["status"]["status_str"] == "error" { job.status = "failed".into(); return Err(invalid("ComfyUI 生图失败")); }
            let mut outputs = entry["outputs"].as_object().into_iter().flatten()
                .filter(|(key, _)| job.output_node_id.as_ref().is_none_or(|selected| *key == selected))
                .flat_map(|(_, output)| output["images"].as_array().into_iter().flatten());
            if let Some(output) = outputs.next() {
                let name = field(output,"filename")?;
                let route = format!("/view?filename={}&subfolder={}&type={}",video_queue::escape(name),
                    video_queue::escape(output["subfolder"].as_str().unwrap_or_default()),
                    video_queue::escape(output["type"].as_str().unwrap_or("output")));
                let (status, _, bytes) = video_queue::http(&profile,"GET",&route,"application/octet-stream",Vec::new()).await?;
                if !status.is_success() || bytes.len() > 48 * 1024 * 1024 { return Err(invalid("生图文件读取失败或过大")); }
                if let Err(failure) = image_format(&bytes) {
                    job.status = "failed".into();
                    return Err(failure);
                }
                return archive(app, job, &bytes);
            }
            if entry["status"]["completed"] == true { job.status = "failed".into(); return Err(invalid("指定工作流输出没有图片")); }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(invalid("等待图片超时，可继续查询原任务"))
}

async fn worker(app: App, mut job: Job) {
    match generate(&app, &mut job).await {
        Ok(()) => job.status = "succeeded".into(),
        Err(error) => {
            job.status = match job.status.as_str() {
                "submitting" => "unknown", "waiting" if job.prompt_id.is_some() => "paused", _ => "failed",
            }.into();
            job.error = Some(error.1);
        }
    }
    if let Ok(mut active) = app.video_active.lock() {
        if let Ok(_guard) = app.writes.lock() { let _ = save(&app, &job); }
        if active.as_ref() == Some(&active_key(&job.project_id,&job.request_id)) { *active = None; }
    }
}

fn attach(app: &App, input: &Value) -> Result<Value> {
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let id = field(input,"projectId")?;
    let (file, mut project) = studio_mcp::project_by_id(app,id)?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;
    let asset = field(input,"assetId")?;
    video_queue::media_path(app,&file,asset,"image")?;
    let kind = field(input,"targetType")?;
    if matches!(kind,"shot-first"|"shot-last") { generation_review::validate_frame(app,&file,&project,asset)?; }
    apply_asset(&mut project,kind,field(input,"targetId")?,asset,true)?;
    persist_project(&file,&mut project)?;
    Ok(json!({"projectId":id,"assetId":asset,"updatedAt":project["updatedAt"]}))
}

fn request_id(input: &Value) -> Result<&str> {
    let id = field(input,"requestId")?;
    if id.len() != 36 || !id.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
    }) { return Err(invalid("请求 ID 须为 UUID")); }
    Ok(id)
}

pub(super) async fn crop_first_frame(app: &App, input: &Value) -> Result<Value> {
    let project_id = field(input,"projectId")?;
    let request = request_id(input)?;
    let id = format!("crop_{request}");
    let (file, project) = studio_mcp::project_by_id(app,project_id)?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    let shot_id = project_authoring::resolve_shot_id(&project,field(input,"shotId")?)?;
    let shot = entity(&project,"shot-first",&shot_id)?;
    let requested = input["sourceAssetId"].as_str();
    let layout_grid = |layout: &Value| -> Option<(u64,u64)> {
        let rows = layout["rows"].as_u64()?;
        let cols = layout["columns"].as_u64()?;
        ((1..=6).contains(&rows) && (1..=6).contains(&cols) && rows*cols>1).then_some((rows,cols))
    };
    let (source,rows,cols) = if let Some(board) = shot["boardAssetId"].as_str().filter(|board| requested.is_none_or(|id| id == *board)) {
        let (rows,cols) = layout_grid(&shot["boardPromptLayout"]).unwrap_or((2,2));
        (board,rows,cols)
    } else {
        let (rows,cols) = layout_grid(&shot["imagePromptLayout"]).ok_or_else(|| invalid("该镜没有已标记分格的看板或关键帧"))?;
        let image = shot["imageAssetId"].as_str().filter(|image| requested.is_none_or(|id| id == *image))
            .ok_or_else(|| invalid("请选择此镜已选用的分格素材"))?;
        (image,rows,cols)
    };
    let panel = input["panel"].as_u64().filter(|panel| (1..=rows*cols).contains(panel))
        .ok_or_else(|| invalid("格号须在图中，按从左到右、从上到下从 1 编号"))?;
    let operation = json!({"shotId":shot_id,"sourceAssetId":source,"panel":panel,"rows":rows,"columns":cols});
    if let Some(asset) = studio_mcp::manifest(app,&file)?.as_array().into_iter().flatten().find(|asset| asset["id"] == id) {
        if asset["frameCrop"] != operation { return Err(invalid("同一个 requestId 已用于不同裁切参数")); }
        video_queue::media_path(app,&file,&id,"image")?;
        return Ok(json!({"assetId":id,"selected":shot["firstFrameAssetId"]==id,"reused":true,
            "updatedAt":revision,"crop":operation}));
    }
    validate_project_revision(input,revision,true)?;
    let path = video_queue::media_path(app,&file,source,"image")?;
    let metadata = fs::metadata(&path)?;
    let ffmpeg = env::var_os("DRAMLO_FFMPEG_BIN").map(PathBuf::from).filter(|bin| bin.is_file())
        .ok_or_else(|| invalid("请配置本地 DRAMLO_FFMPEG_BIN 后裁切看板"))?;
    let scratch = tempfile::tempdir()?;
    let output = scratch.path().join("frame.png");
    let (column,row) = ((panel-1)%cols,(panel-1)/cols);
    let filter = format!("crop=w=floor(iw/{cols}):h=floor(ih/{rows}):x=floor(iw*{column}/{cols}):y=floor(ih*{row}/{rows}):exact=1");
    let mut command = tokio::process::Command::new(ffmpeg);
    command.kill_on_drop(true).args(["-v","error","-nostdin","-i"]).arg(&path)
        .args(["-vf",&filter,"-frames:v","1","-threads","1"]).arg(&output)
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output_status = tokio::time::timeout(Duration::from_secs(15),command.output()).await
        .map_err(|_| invalid("本地裁切超时"))??;
    if !output_status.status.success() { return Err(invalid("无法读取看板或裁切所选分格")); }
    let bytes = fs::read(output)?;
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let (latest_file,mut latest) = studio_mcp::project_by_id(app,project_id)?;
    let current = latest["updatedAt"].as_u64();
    if latest_file != file || current != Some(revision) { return Err(Failure::revision_conflict(current,Some(revision))); }
    let checked = fs::metadata(video_queue::media_path(app,&file,source,"image")?)?;
    if checked.len() != metadata.len() || checked.modified().ok() != metadata.modified().ok() {
        return Err(invalid("裁切期间看板文件已变更，请重新确认分格"));
    }
    store_image(&file,&id,&bytes,json!({"source":"derived","projectId":project_id,"frameCrop":operation}))?;
    apply_asset(&mut latest,"shot-first",&shot_id,&id,true)?;
    persist_project(&file,&mut latest)?;
    Ok(json!({"assetId":id,"selected":true,"reused":false,"updatedAt":latest["updatedAt"],"crop":operation}))
}

fn import(app: &App, input: &Value) -> Result<Value> {
    const MAX_IMAGE: usize = 8 * 1024 * 1024;
    let code = field(input,"dataBase64")?;
    if code.len() > MAX_IMAGE.div_ceil(3) * 4 { return Err(invalid("图片超过 8 MB")); }
    let bytes = STANDARD.decode(code).map_err(|_| invalid("图片 Base64 无效"))?;
    if bytes.len() > MAX_IMAGE { return Err(invalid("图片超过 8 MB")); }
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let project = field(input,"projectId")?;
    let (path, mut data) = studio_mcp::project_by_id(app,project)?;
    let id = format!("import_{}",request_id(input)?);
    if store_image(&path,&id,&bytes,json!({"source":"uploaded","projectId":project}))? {
        persist_project(&path,&mut data)?;
    }
    Ok(json!({"assetId":id,"projectId":project,"updatedAt":data["updatedAt"]}))
}

#[derive(Deserialize)]
pub(super) struct BrowserCall { name: String, input: Value }

pub(super) async fn browser(State(app): State<App>, Json(request): Json<BrowserCall>) -> Result<Json<Value>> {
    if !METHODS.contains(&request.name.as_str()) { return Err(invalid("未知生图操作")); }
    Ok(Json(call(&app,&request.name,&request.input)?))
}

/// 列出未结的图像预检与视频批次。
///
/// unknown 必须核对供应商后台才能关闭；prepared 可以直接撤销。
/// 超过一天的 prepared 标 stale，但不自动作废——预检本身不花钱，
/// 自动处理反而可能误删用户还在准备中的计划。
pub(super) fn pending(app: &App, input: &Value) -> Result<Value> {
    let project = input["projectId"].as_str();
    let scope = input["kind"].as_str().unwrap_or("all");
    if !matches!(scope, "all" | "image" | "video") {
        return Err(invalid("kind 只能是 all、image 或 video"));
    }
    let dir = state_directory(&app.root)?;
    let now = SystemTime::now();
    let mut image = Vec::new();
    let mut video = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(value) = studio_mcp::read_json(&path) else { continue };
        if !value.is_object() { continue; }
        if project.is_some_and(|id| value["projectId"].as_str() != Some(id)) {
            continue;
        }
        let status = value["status"].as_str().unwrap_or("");
        if !matches!(status, "prepared" | "starting" | "submitting" | "waiting" | "paused" | "unknown") {
            continue;
        }
        let age = fs::metadata(&path)
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| now.duration_since(time).ok())
            .map(|elapsed| elapsed.as_secs() / 60)
            .unwrap_or(0);
        let is_image = value["targetType"].is_string();
        if (scope == "image" && !is_image) || (scope == "video" && is_image) {
            continue;
        }
        let mut item = json!({
            "projectId": value["projectId"],
            "requestId": value["requestId"],
            "status": status,
            "ageMinutes": age,
            "stale": status == "prepared" && age >= 24 * 60,
        });
        if is_image {
            item["targetType"] = value["targetType"].clone();
            item["targetId"] = value["targetId"].clone();
            item["prompt"] = json!(value["prompt"].as_str().unwrap_or("").chars().take(60).collect::<String>());
            image.push(item);
        } else if value["steps"].is_array() {
            item["shotCount"] = json!(value["steps"].as_array().map(Vec::len).unwrap_or(0));
            video.push(item);
        }
    }
    let by_age = |left: &Value, right: &Value| {
        right["ageMinutes"].as_u64().cmp(&left["ageMinutes"].as_u64())
    };
    image.sort_by(by_age);
    video.sort_by(by_age);
    Ok(json!({
        "projectId": project,
        "pendingImage": image,
        "pendingVideo": video,
        "total": image.len() + video.len(),
    }))
}

pub(super) fn call(app: &App, name: &str, input: &Value) -> Result<Value> {
    if name == "studio_pending_jobs" { return pending(app,input); }
    if name == "studio_describe_image_inputs" { return describe_inputs(app,input); }
    if name == "studio_attach_image_asset" { return attach(app,input); }
    if name == "studio_import_image_asset" { return import(app,input); }
    let id = field(input,"projectId")?;
    let mut active = app.video_active.lock().map_err(|_| invalid("制作队列锁不可用"))?;
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    if name == "studio_current_image_job" {
        let kind = field(input,"targetType")?;
        target_slot(kind)?;
        let mut result = latest(app,id,kind,field(input,"targetId")?)?;
        if let Some(job) = &mut result { normalized(job,&active); }
        return Ok(json!({"job":result.as_ref().filter(|job| job.status != "discarded").map(response)}));
    }
    let request = request_id(input)?;
    let path = job_path(app,id,request)?;
    let mut job = match read(&path) {
        Ok(job) => job,
        Err(_) if name == "studio_prepare_image" && !path.exists() => {
            let job = prepare(app,input,id,request,&[])?;
            if let Some(mut previous) = latest(app,id,&job.target_type,&job.target_id)? {
                normalized(&mut previous,&active);
                if matches!(previous.status.as_str(),"starting"|"submitting"|"waiting"|"paused"|"unknown") {
                    return Err(invalid(&format!(
                        "此目标还有未结生图任务（状态 {}，请求 {}）。unknown 须核对供应商后用 studio_resolve_unknown_image 关闭；用 studio_pending_jobs 查看全部未结任务",
                        previous.status, previous.request_id)));
                }
                // 旧的未确认预检直接作废，避免残留计划一直挂在统计里。
                if previous.status == "prepared" {
                    previous.status = "superseded".into();
                    save(app,&previous)?;
                }
            }
            save(app,&job)?;
            atomic_save(&latest_path(app,id,&job.target_type,&job.target_id)?,request.as_bytes())?;
            return Ok(json!({"job":response(&job),"reused":false}));
        }
        Err(e) => return Err(e),
    };
    let key: Lock = Some(active_key(id,request));
    normalized(&mut job,&active);
    if name == "studio_resolve_unknown_image" {
        if input["confirmed"] != true { return Err(invalid("须由用户确认已核对供应商记录")); }
        let reused = job.unknown_resolution.is_some();
        if !reused {
            if job.status != "unknown" { return Err(invalid("只能关闭结果未知且已核对的生图请求")); }
            let reason = field(input,"reason")?;
            if reason.encode_utf16().count() > 500 { return Err(invalid("核对说明不能超过 500 字符")); }
            job.unknown_resolution = Some(json!({"previousStatus":"unknown","reason":reason,
                "resolvedAt":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64}));
            job.status = "discarded".into();
            save(app,&job)?;
        }
        return Ok(json!({"job":response(&job),"reused":reused}));
    }
    if name == "studio_discard_image_plan" {
        let reused = job.status == "discarded";
        if !reused {
            if job.status != "prepared" || job.prompt_id.is_some() {
                return Err(invalid("只能撤销尚未提交的预检计划"));
            }
            job.status = "discarded".into();
            save(app,&job)?;
        }
        return Ok(json!({"job":response(&job),"reused":reused}));
    }
    if matches!(name,"studio_confirm_image" | "studio_resume_image") {
        if input["confirmed"] != true { return Err(invalid("请先确认生图计划")); }
        if job.status == "discarded" { return Err(invalid("此计划已撤销，请用新请求 ID 重新预检")); }
        let expected = if name == "studio_confirm_image" { "prepared" } else { "paused" };
        if job.status == expected {
            if active.is_some() { return Err(Failure::new(StatusCode::LOCKED, "本机已有制作任务运行".into())); }
            if latest(app,id,&job.target_type,&job.target_id)?.is_none_or(|latest| latest.request_id != job.request_id) {
                return Err(invalid("已有较新的生图计划，请读取当前任务"));
            }
            if expected == "prepared" {
                let (file,project) = studio_mcp::project_by_id(app,id)?;
                check_approval(app,&job,&file,&project)?;
            }
            if profile_stamp(&profile(app,Some(&job.profile_id))?) != job.profile_stamp {
                return Err(invalid("图像接入已变化，请重新预检"));
            }
            job.status = if expected == "prepared" { "starting" } else { "waiting" }.into();
            job.error = None;
            save(app,&job)?;
            *active = key;
            tokio::spawn(worker(app.clone(),job.clone()));
            return Ok(json!({"job":response(&job),"reused":false}));
        }
    }
    Ok(json!({"job":response(&job),"reused":true}))
}
