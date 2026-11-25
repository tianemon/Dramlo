use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const METHODS: &[&str] = &[
    "studio_create_project", "studio_rename_project", "studio_import_script", "studio_update_script",
    "studio_upsert_project_assets", "studio_append_shots", "studio_replace_shots", "studio_revise_shot",
    "studio_bind_shot_asset", "studio_timeline", "studio_edit_timeline", "studio_project_settings",
    "studio_update_project_defaults", "studio_reorder_shots", "studio_remove_shot", "studio_update_shot_inputs",
    "studio_set_asset_prompt", "studio_batch_edit_shots", "studio_approve_script",
];

pub(super) const PROJECT_INSTRUCTIONS: &str = "AGENTS.md";

pub(super) fn initialize_project_instructions(folder: &Path) -> Result<()> {
    use std::io::Write;

    let path = folder.join(PROJECT_INSTRUCTIONS);
    match fs::symlink_metadata(&path) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut file = tempfile::NamedTempFile::new_in(folder)?;
    file.write_all(include_bytes!("../templates/project-agents.md"))?;
    file.as_file().sync_all()?;
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.error.into()),
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn required<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field].as_str().filter(|text| !text.trim().is_empty()).ok_or_else(|| invalid("缺少必要字段"))
}

fn script_text<'a>(input: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    let text = required(input, key)?;
    if text.encode_utf16().count() > limit { return Err(invalid("剧本字段过长")); }
    Ok(text)
}

fn list<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>> {
    if value[key].is_null() { value[key] = json!([]); }
    value[key].as_array_mut().ok_or_else(|| invalid("项目列表格式无效"))
}

fn save_project(path: &Path, value: &Value) -> Result<()> {
    atomic_save(path, &serde_json::to_vec_pretty(value).map_err(|_| invalid("无法保存项目"))?)
}

fn create(app: &App, input: &Value) -> Result<Value> {
    let name = required(input, "name")?.trim();
    if name.chars().count() > 100 { return Err(invalid("项目名过长")); }
    let timestamp = now();
    let unique = tempfile::Builder::new().prefix("project__p_").tempdir_in(&app.root)?;
    let folder_name = unique.path().file_name().and_then(|name| name.to_str())
        .ok_or_else(|| invalid("项目目录无效"))?.to_owned();
    let id = folder_name.trim_start_matches("project__");
    let path = scoped_path(&app.root, &unique.path().join("project.json"))?;
    let settings: Value = serde_json::from_str(include_str!("../../src/types/projectDefaults.json"))
        .map_err(|_| invalid("项目默认设置无效"))?;
    let project = json!({
        "id": id, "name": name, "script": null, "shots": [], "subjects": [], "scenes": [], "props": [], "timeline": [],
        "settings": settings,
        "canvas": { "nodes": [], "edges": [], "groups": [], "viewport": { "x": 0, "y": 0, "zoom": 1 } },
        "agentState": { "autonomy": "gated", "messages": [], "tasks": [], "callBudget": 24 },
        "createdAt": timestamp, "updatedAt": timestamp
    });
    initialize_project_instructions(unique.path())?;
    save_project(&path, &project)?;
    let _folder = unique.keep();
    studio_mcp::set_active_project(app, id)
}

fn import(project: &mut Value, input: &Value) -> Result<()> {
    if project["script"]["content"].as_str().is_some_and(|text| !text.trim().is_empty()) ||
        project["chapters"].as_array().is_some_and(|items| items.iter().any(|item| item["content"].as_str().is_some_and(|text| !text.trim().is_empty()))) {
        return Err(invalid("项目已有剧本，请使用剧本更新"));
    }
    let title = required(input, "title")?;
    let content = script_text(input, "content", 50_000)?;
    let timestamp = now();
    project["script"] = json!({
        "id": format!("s_{timestamp}"), "topic": title, "content": content, "origin": "imported",
        "keywords": [], "style": "cinematic", "playReady": true, "aspectRatio": project["settings"]["aspectRatio"],
        "shotDuration": project["settings"]["defaultShotDuration"], "createdAt": timestamp
    });
    if let Some(chapters) = input["chapters"].as_array() {
        let mut ids = std::collections::HashSet::new();
        if chapters.len() > 25 { return Err(invalid("章节过多")); }
        let mut result = Vec::new();
        for (index, chapter) in chapters.iter().enumerate() {
            let id = required(chapter, "id")?;
            if !ids.insert(id) { return Err(invalid("章节 ID 重复")); }
            result.push(json!({ "id": id, "title": required(chapter, "title")?,
                "content": script_text(chapter, "content", 50_000)?, "order": index, "status": "ready",
                "createdAt": timestamp, "updatedAt": timestamp }));
        }
        project["chapters"] = json!(result);
    }
    project["playApproved"] = json!(false);
    Ok(())
}

fn asset_cards(project: &mut Value, input: &Value) -> Result<()> {
    let fields = input.as_object().ok_or_else(|| invalid("资产卡片请求必须是对象"))?;
    if let Some(key) = fields.keys().find(|key| !matches!(key.as_str(), "projectId" | "expectedUpdatedAt" | "items")) {
        return Err(invalid(&format!("资产卡片请求不支持字段「{key}」")));
    }
    let updates = input["items"].as_array().filter(|items| !items.is_empty() && items.len() <= 25)
        .ok_or_else(|| invalid("请提供资产卡片"))?;
    let mut ids = std::collections::HashSet::new();
    for update in updates {
        let id = required(update, "id")?;
        if !ids.insert(id) { return Err(invalid("资产 ID 重复")); }
        let (collection, node_type, key) = match required(update, "type")? {
            "subject" => ("subjects", "subject", "subjectId"),
            "scene" => ("scenes", "scene", "sceneId"),
            "prop" => ("props", "prop", "propId"),
            _ => return Err(invalid("资产类型无效")),
        };
        let fields = update.as_object().ok_or_else(|| invalid("资产卡片必须是对象"))?;
        if let Some(field) = fields.keys().find(|field| {
            !matches!(field.as_str(), "id" | "type" | "name" | "description")
                && !(collection == "subjects" && matches!(field.as_str(), "kind" | "promptName"))
        }) {
            return Err(invalid(&format!("资产卡片「{id}」不支持字段「{field}」")));
        }
        let cards = list(project, collection)?;
        let mut card = cards.iter().find(|item| item["id"] == id).cloned().unwrap_or_else(|| json!({ "id": id }));
        card["name"] = json!(required(update, "name")?);
        card["description"] = json!(required(update, "description")?);
        if collection == "subjects" {
            let kind = required(update, "kind")?;
            if !["person", "animal", "other"].contains(&kind) { return Err(invalid("人物资产类型无效")); }
            card["kind"] = json!(kind);
            if let Some(name) = update.get("promptName") {
                if name.is_null() { card.as_object_mut().unwrap().remove("promptName"); }
                else {
                    let name = name.as_str().map(str::trim).filter(|name|
                        !name.is_empty() && name.encode_utf16().count() <= 80 && !name.chars().any(char::is_control))
                        .ok_or_else(|| invalid("人物提示词名称须为 1 至 80 字，或填写 null 清除"))?;
                    card["promptName"] = json!(name);
                }
            }
        }
        if let Some(existing) = cards.iter_mut().find(|item| item["id"] == id) { *existing = card.clone(); }
        else { cards.push(card.clone()); }
        let mut data = json!({ "label": card["name"] });
        data[key] = json!(id);
        add_canvas_node(project, format!("{node_type}:{id}"), node_type, data)?;
    }
    Ok(())
}

fn add_canvas_node(project: &mut Value, id: String, kind: &str, data: Value) -> Result<()> {
    let nodes = list(&mut project["canvas"], "nodes")?;
    if let Some(node) = nodes.iter_mut().find(|node| node["id"] == id) {
        if node["type"] == kind { node["data"]["label"] = data["label"].clone(); }
        return Ok(());
    }
    let y = nodes.iter().filter_map(|node| node["position"]["y"].as_f64()).fold(-340.0, f64::max) + 340.0;
    nodes.push(json!({ "id": id, "type": kind, "position": { "x": 0, "y": y }, "data": data, "status": "idle" }));
    Ok(())
}

fn append(project: &mut Value, input: &Value) -> Result<Value> {
    if project["continuousCaptureEnabled"] == true { return Err(invalid("连续拍项目需要结构化拍摄计划")); }
    if !project["script"]["content"].as_str().is_some_and(|text| !text.trim().is_empty()) {
        return Err(invalid("请先导入剧本"));
    }
    let scene = required(input, "sceneId")?;
    let chapter = input["chapterId"].as_str();
    let present = |collection: &str, id: &str| project[collection].as_array().is_some_and(|items| items.iter().any(|item| item["id"] == id));
    if !present("scenes", scene) || chapter.is_some_and(|id| !present("chapters", id)) {
        return Err(invalid("找不到场景或章节"));
    }
    let drafts = input["shots"].as_array().filter(|items| !items.is_empty() && items.len() <= 12)
        .ok_or_else(|| invalid("请提供分镜草稿"))?;
    let mut ids = std::collections::HashSet::new();
    for draft in drafts {
        let id = required(draft, "id")?;
        if !ids.insert(id) || present("shots", id) { return Err(invalid("分镜 ID 重复")); }
        required(draft, "title")?;
        required(draft, "visualPrompt")?;
        if !draft["duration"].as_f64().is_some_and(|value| value > 0.0 && value <= 120.0) {
            return Err(invalid("镜头时长无效"));
        }
        for (slot, collection) in [("subjectIds", "subjects"), ("propIds", "props")] {
            if draft[slot].as_array().is_some_and(|items| items.iter().any(|id| id.as_str().is_none_or(|id| !present(collection, id)))) {
                return Err(invalid("分镜引用未定义的资产"));
            }
        }
    }
    let mut index = list(project, "shots")?.iter().filter_map(|shot| shot["index"].as_u64()).max().map_or(0, |value| value.saturating_add(1));
    let mut result = Vec::new();
    for draft in drafts {
        let mut shot = draft.clone();
        if let Some(prompt) = draft.get("compiledVideoPrompt") {
            if !prompt.as_str().is_some_and(|text| !text.trim().is_empty() && text.encode_utf16().count() <= 12000) {
                return Err(invalid("最终视频提示词无效"));
            }
        }
        shot["index"] = json!(index);
        shot["sceneId"] = json!(scene);
        if let Some(chapter) = chapter { shot["chapterId"] = json!(chapter); }
        let id = required(&shot, "id")?;
        let target = format!("shotVideo:{id}");
        add_canvas_node(project, target.clone(), "shotVideo", json!({
            "shotId": id, "label": format!("{}. {}", index + 1, required(&shot, "title")?)
        }))?;
        let source = format!("scene:{scene}");
        let graph = &mut project["canvas"];
        if list(graph, "nodes")?.iter().any(|node| node["id"] == source) {
            let edge_id = format!("e-scene-{scene}-{target}");
            let edges = list(graph, "edges")?;
            if !edges.iter().any(|edge| edge["id"] == edge_id) {
                edges.push(json!({ "id": edge_id, "source": source, "target": target }));
            }
        }
        list(project, "shots")?.push(shot.clone());
        result.push(shot);
        index = index.saturating_add(1);
    }
    Ok(json!({ "shots": result }))
}

pub(super) fn changes_shot_order(name: &str) -> bool {
    matches!(name, "studio_reorder_shots" | "studio_remove_shot" | "studio_replace_shots")
}

pub(super) fn image_consistency(shot: &Value) -> &'static str {
    match (shot["imageAssetId"].as_str().filter(|id| !id.is_empty()),shot["firstFrameAssetId"].as_str().filter(|id| !id.is_empty())) {
        (Some(image),Some(first)) if image == first => "same",
        (Some(_),Some(_)) => "diverged",
        (Some(_),None) => "image_only",
        (None,Some(_)) => "first_only",
        _ => "empty",
    }
}

pub(super) fn missing_prop_images(project: &Value, shot: &Value) -> Vec<Value> {
    let names = ["title", "visualPrompt", "videoPrompt", "propFocus", "blocking", "purpose"].iter()
        .filter_map(|field| shot[*field].as_str()).collect::<Vec<_>>().join("\n");
    project["props"].as_array().into_iter().flatten().filter(|prop| {
        let referenced = if let Some(bound) = shot["propIds"].as_array() {
            bound.contains(&prop["id"])
        } else { prop["name"].as_str().is_some_and(|name| !name.is_empty() && names.contains(name)) };
        referenced && prop["designSheetAssetId"].as_str().is_none_or(str::is_empty)
    }).map(|prop| json!({"id":prop["id"],"name":prop["name"]})).collect()
}

fn shot_inputs(project: &Value, shot: &Value, node: Option<&Value>) -> Value {
    let audio = shot["audioMode"].as_str().unwrap_or_else(|| {
        if shot["voiceover"].as_str().is_some_and(|text| !text.trim().is_empty()) { "h3-dialogue" }
        else { "none" }
    });
    let canvas = node.map(|node| &node["data"]);
    let still = canvas.and_then(|data| data["videoStillMode"].as_str()).filter(|mode| matches!(*mode,"identity"|"keyframe"|"storyboard"))
        .unwrap_or_else(|| if canvas.is_some_and(|data| data["videoModel"] == "wan-prime") { "keyframe" } else { "identity" });
    let frames = shot["h3GenMode"] == "fl2va" || project["settings"]["h3ShotChainMode"] == "fl2va";
    json!({"effectiveAudioMode":audio,"subjectBindingMode":shot["subjectBindingMode"].as_str().unwrap_or("infer"),
        "subjectIds":shot["subjectIds"].as_array().cloned().unwrap_or_default(),"stillMode":still,
        "canvasInputMode":if frames {"first-last-frame"} else {"reference"},
        "firstFrameAssetId":shot["firstFrameAssetId"],"lastFrameAssetId":shot["lastFrameAssetId"],
        "videoProfileId":shot["videoProfileId"],
        "duration":shot["duration"],
        "missingPropImages":missing_prop_images(project,shot)})
}

pub(super) fn shot_timing_warnings(shot: &Value) -> Vec<String> {
    let Some(duration) = shot["duration"].as_f64().filter(|value| *value > 0.0) else { return vec![]; };
    let Some(prompt) = shot["compiledVideoPrompt"].as_str() else { return vec![]; };
    fn seconds(text: &str) -> Option<f64> {
        let parts: Vec<_> = text.trim().split(':').collect();
        if !(2..=3).contains(&parts.len()) { return None; }
        let tail = parts[parts.len()-1].parse::<f64>().ok().filter(|value| (0.0..60.0).contains(value))?;
        let minutes = parts[parts.len()-2].parse::<u32>().ok()?;
        if parts.len() == 2 { return Some(f64::from(minutes)*60.0+tail); }
        let hours = parts[0].parse::<u32>().ok()?;
        (minutes < 60).then(|| f64::from(hours)*3600.0+f64::from(minutes)*60.0+tail)
    }
    let mut warnings = Vec::new();
    for part in prompt.split(['[','【']).skip(1) {
        let Some((range,_)) = part.split_once([']','】']) else { continue; };
        let Some((start,end)) = range.split_once(['-','–','—','~','～','至']) else { continue; };
        let (Some(first),Some(last)) = (seconds(start),seconds(end)) else { continue; };
        if last <= first || last > duration+0.05 {
            warnings.push(format!("按镜头内时间计算，提示词节拍 [{range}] 与镜头时长 {duration} 秒不一致，请核对时间码。"));
        }
        if warnings.len() >= 8 { break; }
    }
    warnings
}

pub(super) fn timed_shots(project: &Value) -> impl Iterator<Item = (&Value, f64, f64)> {
    let mut shots: Vec<_> = project["shots"].as_array().into_iter().flatten().collect();
    shots.sort_by_key(|shot| shot["index"].as_u64().unwrap_or(u64::MAX));
    let mut start = 0.0;
    shots.into_iter().map(move |shot| {
        let end = start + shot["duration"].as_f64().filter(|duration| *duration > 0.0).unwrap_or(0.0);
        let result = (shot, start, end);
        start = end;
        result
    })
}

pub(super) fn timed_title(title: &str, number: usize, start: f64, end: f64) -> Option<String> {
    fn clock_marker(value: &str) -> bool {
        let Some((minutes,seconds)) = value.trim().split_once(':') else { return false; };
        !minutes.is_empty() && minutes.bytes().all(|ch| ch.is_ascii_digit()) &&
            seconds.len() == 2 && seconds.parse::<u8>().is_ok_and(|seconds| seconds < 60)
    }
    fn clock(seconds: f64) -> String {
        let seconds = seconds.round() as u64;
        format!("{}:{:02}",seconds / 60,seconds % 60)
    }
    let mut parts = title.splitn(3,'｜');
    let prefix = parts.next()?;
    let span = parts.next()?;
    let label = parts.next()?.trim();
    let (from,to) = span.split_once(['-','–','—'])?;
    if prefix.parse::<usize>().is_err() || !clock_marker(from) || !clock_marker(to) || label.is_empty() { return None; }
    Some(format!("{number:02}｜{}-{}｜{label}",clock(start),clock(end)))
}

fn refresh_timed_titles(project: &mut Value) {
    let titles: std::collections::HashMap<_,_> = timed_shots(project).enumerate()
        .filter_map(|(index,(shot,start,end))| Some((shot["id"].as_str()?.to_owned(),
            timed_title(shot["title"].as_str()?,index+1,start,end)?))).collect();
    for shot in project["shots"].as_array_mut().into_iter().flatten() {
        if let Some(title) = shot["id"].as_str().and_then(|id| titles.get(id)) { shot["title"] = json!(title); }
    }
}

pub(super) fn shot_node_label(shot: &Value, ordinal: usize) -> String {
    let number = shot["index"].as_u64().unwrap_or(ordinal as u64).saturating_add(1);
    let title = shot["title"].as_str().unwrap_or("").trim();
    if title.is_empty() { return format!("第 {number} 镜"); }
    let already_numbered = title.split_once(['｜', '|']).is_some_and(|(prefix, _)| {
        let prefix = prefix.trim_end();
        !prefix.is_empty() && prefix.bytes().all(|ch| ch.is_ascii_digit())
    });
    if already_numbered { title.to_owned() } else { format!("{number}. {title}") }
}

pub(super) fn shot_node_labels(project: &Value) -> std::collections::HashMap<String, String> {
    project["shots"].as_array().into_iter().flatten().enumerate()
        .filter_map(|(ordinal, shot)| Some((shot["id"].as_str()?.to_owned(), shot_node_label(shot, ordinal))))
        .collect()
}

fn sync_shot_node_labels(project: &mut Value) {
    let labels = shot_node_labels(project);
    for node in project.pointer_mut("/canvas/nodes").and_then(Value::as_array_mut).into_iter().flatten() {
        if !matches!(node["type"].as_str(), Some("shotBoard" | "shotImage" | "shotFrames" | "shotVideo" | "audio")) { continue; }
        if let Some(label) = node["data"]["shotId"].as_str().and_then(|id| labels.get(id)) {
            node["data"]["label"] = json!(label);
        }
    }
}

fn shot_change(project: &mut Value, name: &str, input: &Value, context: &Value) -> Result<Value> {
    let before: Vec<String> = timed_shots(project).map(|(shot, _, _)| required(shot, "id").map(str::to_owned)).collect::<Result<_>>()?;
    if before.iter().collect::<std::collections::HashSet<_>>().len() != before.len() {
        return Err(invalid("项目有重复的镜头 ID，请先修复"));
    }
    let removing = if name == "studio_remove_shot" { Some(required(input, "shotId")?) } else { None };
    let after: Vec<String> = if let Some(id) = removing {
        if !before.iter().any(|item| item == id) { return Err(invalid("找不到指定镜头")); }
        before.iter().filter(|item| *item != id).cloned().collect()
    } else {
        let ids = input["shotIds"].as_array().ok_or_else(|| invalid("请提供完整镜头顺序"))?;
        let order = ids.iter().map(|id| id.as_str().map(str::to_owned).ok_or_else(|| invalid("镜头 ID 无效")))
            .collect::<Result<Vec<_>>>()?;
        let expected: std::collections::HashSet<_> = before.iter().collect();
        if order.len() != before.len() || order.iter().collect::<std::collections::HashSet<_>>() != expected {
            return Err(invalid("顺序须包含项目中的每个镜头且仅出现一次"));
        }
        order
    };
    let predecessor = |id: &String| before.iter().position(|item| item == id).and_then(|i| i.checked_sub(1)).map(|i| &before[i]);
    let review: Vec<_> = after.iter().enumerate().filter_map(|(i, id)| {
        let next = i.checked_sub(1).map(|previous| &after[previous]);
        (predecessor(id) != next).then(|| json!({"shotId":id,"previousShotId":next}))
    }).collect();
    let nodes: Vec<Value> = project["canvas"]["nodes"].as_array().into_iter().flatten()
        .filter(|node| removing.is_some_and(|id| node["data"]["shotId"] == id)).map(|node| node["id"].clone()).collect();
    let edges: Vec<Value> = project["canvas"]["edges"].as_array().into_iter().flatten()
        .filter(|edge| nodes.contains(&edge["source"]) || nodes.contains(&edge["target"])).map(|edge| edge["id"].clone()).collect();
    let clips: Vec<Value> = project["timeline"].as_array().into_iter().flatten()
        .filter(|clip| removing.is_some_and(|id| clip["shotId"] == id)).map(|clip| clip["id"].clone()).collect();
    let mut result = json!({"shotIds":after,"removedShotId":removing,"removedNodeIds":nodes,"removedEdgeIds":edges,
        "removedTimelineClipIds":clips,"continuityReview":review,"queue":context,"applied":false,"assetsRetained":true});
    if input["confirmed"] != true { return Ok(result); }
    if context["blocked"] == true { return Err(invalid("项目有运行、暂停或提交结果未知的制作批次，请先处理")); }
    prune_shot_links(project, &nodes, &removing.into_iter().collect())?;
    apply_shot_order(project, &after)?;
    result["applied"] = json!(true);
    Ok(result)
}

fn prune_shot_links(project: &mut Value, nodes: &[Value], removed: &std::collections::HashSet<&str>) -> Result<()> {
    if !nodes.is_empty() {
        let canvas = &mut project["canvas"];
        list(canvas, "nodes")?.retain(|node| !nodes.contains(&node["id"]));
        for node in list(canvas, "nodes")? {
            if let Some(refs) = node["data"]["referenceSourceNodeIds"].as_array_mut() { refs.retain(|id| !nodes.contains(id)); }
        }
        list(canvas, "edges")?.retain(|edge| !nodes.contains(&edge["source"]) && !nodes.contains(&edge["target"]));
        for group in list(canvas, "groups")? {
            if let Some(items) = group["nodeIds"].as_array_mut() { items.retain(|id| !nodes.contains(id)); }
        }
    }
    if !removed.is_empty() {
        list(project, "timeline")?.retain(|clip| clip["shotId"].as_str().is_none_or(|id| !removed.contains(id)));
    }
    Ok(())
}

fn apply_shot_order(project: &mut Value, order: &[String]) -> Result<()> {
    let mut old: std::collections::HashMap<_, _> = list(project, "shots")?.drain(..)
        .filter_map(|shot| shot["id"].as_str().map(str::to_owned).map(|id| (id, shot))).collect();
    let mut reordered = Vec::new();
    for (index, id) in order.iter().enumerate() {
        let mut item = old.remove(id).ok_or_else(|| invalid("分镜记录不一致"))?;
        if let (Some(previous), Some(title)) = (item["index"].as_u64(), item["title"].as_str()) {
            let old_label = format!("{}. {title}", previous.saturating_add(1));
            for node in project["canvas"]["nodes"].as_array_mut().into_iter().flatten() {
                if node["data"]["shotId"] == id.as_str() && node["data"]["label"] == old_label {
                    node["data"]["label"] = json!(format!("{}. {title}", index + 1));
                }
            }
        }
        item["index"] = json!(index);
        reordered.push(item);
    }
    project["shots"] = json!(reordered);
    Ok(())
}

fn replace_shots(project: &mut Value, input: &Value, context: &Value) -> Result<Value> {
    if project["continuousCaptureEnabled"] == true { return Err(invalid("连续拍项目需要结构化拍摄计划")); }
    let before: Vec<_> = timed_shots(project).map(|(shot,_,_)| required(shot,"id").map(str::to_owned)).collect::<Result<_>>()?;
    let positions: std::collections::HashMap<_,_> = before.iter().enumerate().map(|(i,id)| (id.as_str(),i)).collect();
    if positions.len() != before.len() { return Err(invalid("项目有重复的镜头 ID，请先修复")); }
    let (start,end) = match required(input,"mode")? {
        "insert" if input.get("shotIds").is_none() => {
            let index = match input.get("afterShotId") {
                None => 0,
                Some(id) => id.as_str().and_then(|id| positions.get(id)).map(|index| index+1)
                    .ok_or_else(|| invalid("找不到插入位置"))?,
            };
            (index,index)
        }
        "replace" if input.get("afterShotId").is_none() => match input.get("shotIds") {
            None => (0,before.len()),
            Some(ids) => {
                let ids = ids.as_array().filter(|ids| !ids.is_empty()).ok_or_else(|| invalid("请按原顺序指定连续镜头"))?;
                let start = ids[0].as_str().and_then(|id| positions.get(id)).copied().ok_or_else(|| invalid("镜头范围无效"))?;
                let end = start+ids.len();
                if before.get(start..end).is_none_or(|range| !ids.iter().zip(range).all(|(id,expected)| id == expected.as_str())) {
                    return Err(invalid("请按原顺序指定连续镜头"));
                }
                (start,end)
            }
        },
        _ => return Err(invalid("插入使用 afterShotId，替换使用 shotIds")),
    };
    let drafts = input["shots"].as_array().filter(|shots| !shots.is_empty() && shots.len() <= 60)
        .ok_or_else(|| invalid("请提供 1 至 60 个分镜草稿"))?;
    let mut ids = std::collections::HashSet::new();
    let mut replacement = Vec::new();
    let original = list(project,"shots")?.clone();
    let mut added = Vec::new();
    let mut revised = Vec::new();
    for draft in drafts {
        let id = required(draft,"id")?;
        if !ids.insert(id) || positions.get(id).is_some_and(|i| !(start..end).contains(i)) {
            return Err(invalid("分镜 ID 重复或位于替换范围之外"));
        }
        let scene = required(draft,"sceneId")?;
        let present = |key: &str,id: &str| project[key].as_array().into_iter().flatten().any(|item| item["id"] == id);
        if !present("scenes",scene) || draft.get("chapterId").is_some_and(|id| !id.is_null() && id.as_str().is_none_or(|id| !present("chapters",id))) {
            return Err(invalid("找不到场景或章节"));
        }
        script_text(draft,"title",160)?;
        script_text(draft,"visualPrompt",12_000)?;
        for key in ["videoPrompt","compiledVideoPrompt"] {
            if draft.get(key).is_some() { script_text(draft,key,12_000)?; }
        }
        if !matches!(draft["cameraMovement"].as_str(),Some("static"|"push"|"pull"|"pan"|"tilt"|"orbit")) {
            return Err(invalid("镜头运镜无效"));
        }
        for (key,limit) in [("voiceover",12_000),("audioDescription",4_000)] {
            if draft.get(key).is_some_and(|value| value.as_str().is_none_or(|text| text.encode_utf16().count() > limit)) {
                return Err(invalid("镜头对白或环境音字段无效"));
            }
        }
        if !draft["duration"].as_f64().is_some_and(|seconds| seconds > 0.0 && seconds <= 120.0) {
            return Err(invalid("镜头时长无效"));
        }
        for (key,collection) in [("subjectIds","subjects"),("propIds","props")] {
            if draft.get(key).is_some_and(|value| value.as_array().is_none_or(|items| {
                items.iter().any(|id| id.as_str().is_none_or(|id| !present(collection,id)))
            })) { return Err(invalid("分镜引用未定义的资产")); }
        }
        let existing = original.iter().find(|shot| shot["id"] == id);
        let mut shot = existing.cloned().unwrap_or_else(|| json!({
            "id":id,"voiceover":"","audioDescription":"","subjectIds":[]
        }));
        for key in ["title","visualPrompt","videoPrompt","compiledVideoPrompt","duration","voiceover","audioDescription",
            "cameraMovement","sceneId","chapterId","subjectIds","propIds"] {
            if let Some(value) = draft.get(key) {
                if key == "chapterId" && value.is_null() { shot.as_object_mut().unwrap().remove(key); }
                else { shot[key] = value.clone(); }
            }
        }
        if let Some(existing) = existing {
            if &shot != existing { revised.push(id); }
        } else { added.push(id); }
        replacement.push(shot);
    }
    let removed: std::collections::HashSet<_> = before[start..end].iter().map(String::as_str).filter(|id| !ids.contains(id)).collect();
    let after: Vec<_> = before[..start].iter().cloned().chain(drafts.iter().map(|draft| draft["id"].as_str().unwrap().to_owned()))
        .chain(before[end..].iter().cloned()).collect();
    let nodes: Vec<_> = project["canvas"]["nodes"].as_array().into_iter().flatten()
        .filter(|node| node["data"]["shotId"].as_str().is_some_and(|id| removed.contains(id)))
        .map(|node| node["id"].clone()).collect();
    let edges: Vec<_> = project["canvas"]["edges"].as_array().into_iter().flatten()
        .filter(|edge| nodes.contains(&edge["source"]) || nodes.contains(&edge["target"])).map(|edge| &edge["id"]).collect();
    let clips: Vec<_> = project["timeline"].as_array().into_iter().flatten()
        .filter(|clip| clip["shotId"].as_str().is_some_and(|id| removed.contains(id))).map(|clip| &clip["id"]).collect();
    let removed_in_order: Vec<_> = before.iter().filter(|id| removed.contains(id.as_str())).collect();
    let review: Vec<_> = after.iter().enumerate().filter_map(|(i,id)| {
        let new_previous = i.checked_sub(1).map(|j| &after[j]);
        let previous = positions.get(id.as_str()).and_then(|j| j.checked_sub(1)).map(|j| &before[j]);
        (positions.contains_key(id.as_str()) && (previous != new_previous || revised.contains(&id.as_str()) ||
            new_previous.is_some_and(|id| revised.contains(&id.as_str())))).then(|| json!({"shotId":id,"previousShotId":new_previous}))
    }).collect();
    let mut result = json!({"shotIds":after,"addedShotIds":added,"updatedShotIds":revised,"removedShotIds":removed_in_order,
        "removedNodeIds":nodes,"removedEdgeIds":edges,"removedTimelineClipIds":clips,
        "continuityReview":review,"applied":false,"assetsRetained":true,"queue":context});
    if input["dryRun"] == true || (input["confirmed"] != true && !removed.is_empty()) { return Ok(result); }
    if context["blocked"] == true { return Err(invalid("项目有运行、暂停或提交结果未知的制作批次，请先处理")); }
    prune_shot_links(project,&nodes,&removed)?;
    list(project,"shots")?.retain(|shot| shot["id"].as_str().is_none_or(|id| !ids.contains(id)));
    for shot in replacement {
        let id = required(&shot,"id")?;
        let title = required(&shot,"title")?;
        let scene = required(&shot,"sceneId")?;
        let old = original.iter().find(|shot| shot["id"] == id);
        let old_scene = old.and_then(|shot| shot["sceneId"].as_str());
        let target = format!("shotVideo:{id}");
        let index = shot["index"].as_u64().unwrap_or_else(|| after.iter().position(|item| item == id).unwrap_or_default() as u64);
        if let Some(old_title) = old.and_then(|shot| shot["title"].as_str()) {
            let old_label = format!("{}. {old_title}",index+1);
            for node in list(&mut project["canvas"],"nodes")? {
                if node["data"]["shotId"] == id && node["data"]["label"] == old_label {
                    node["data"]["label"] = json!(format!("{}. {title}",index+1));
                }
            }
        }
        let needs_node = !list(&mut project["canvas"],"nodes")?.iter().any(|node| node["id"] == target);
        if needs_node {
            add_canvas_node(project,target.clone(),"shotVideo",json!({"shotId":id,"label":format!("{}. {title}",index+1)}))?;
        }
        if needs_node || old_scene != Some(scene) {
            let graph = &mut project["canvas"];
            let nodes = list(graph,"nodes")?;
            let source = format!("scene:{scene}");
            let has_source = nodes.iter().any(|node| node["id"] == source);
            let shot_nodes: Vec<_> = nodes.iter().filter(|node| node["data"]["shotId"] == id).filter_map(|node| node["id"].as_str().map(str::to_owned)).collect();
            let edges = list(graph,"edges")?;
            let mut targets = vec![];
            if let Some(old) = old_scene {
                edges.retain(|edge| {
                    let Some(target) = edge["target"].as_str().filter(|target| shot_nodes.iter().any(|node| node == target)) else { return true; };
                    if edge["source"] != format!("scene:{old}") || edge["id"] != format!("e-scene-{old}-{target}") { return true; }
                    targets.push((target.to_owned(),edge["sourceHandle"].clone(),edge["targetHandle"].clone()));
                    false
                });
            }
            if targets.is_empty() { targets.push((target,Value::Null,Value::Null)); }
            if has_source {
                for (target,from,to) in targets {
                    let id = format!("e-scene-{scene}-{target}");
                    if !edges.iter().any(|edge| edge["id"] == id) {
                        let mut edge = json!({"id":id,"source":source,"target":target});
                        if !from.is_null() { edge["sourceHandle"] = from; }
                        if !to.is_null() { edge["targetHandle"] = to; }
                        edges.push(edge);
                    }
                }
            }
        }
        let mut shot = shot;
        shot["index"] = json!(index);
        list(project,"shots")?.push(shot);
    }
    apply_shot_order(project,&after)?;
    result["applied"] = json!(true);
    Ok(result)
}

pub(super) fn timeline_asset<'a>(project: &'a Value, clip: &'a Value) -> Option<&'a str> {
    clip["assetId"].as_str().or_else(|| {
        let shot = project["shots"].as_array()?.iter().find(|shot| shot["id"] == clip["shotId"])?;
        shot["videoAssetId"].as_str().or_else(|| shot["imageAssetId"].as_str())
    })
}

fn edit_timeline(app: &App, file: &Path, project: &mut Value, input: &Value) -> Result<Value> {
    if input.get("clips").is_some() && input["buildFromShots"] == true {
        return Err(invalid("手工片段与自动组装只能选一项"));
    }
    if input["buildFromShots"] != true && ["clips","captionSrt","bgm","overlays"].iter().all(|key| input.get(key).is_none()) {
        return Err(invalid("请提供时间线修改"));
    }
    let supplied = input.get("clips").map(|clips| clips.as_array().filter(|clips| clips.len()<=500)
        .cloned().ok_or_else(|| invalid("时间线片段列表无效"))).transpose()?;
    let clips = if input["buildFromShots"] == true {
        Some(timeline_from_shots(project))
    } else { supplied };
    if let Some(clips) = clips {
        let mut ids = std::collections::HashSet::new();
        for clip in &clips {
            let id = required(clip,"id")?;
            required(clip,"shotId")?;
            let valid = |key: &str,min: f64,max: f64| clip[key].as_f64().is_some_and(|value| (min..=max).contains(&value));
            if !ids.insert(id) || !valid("startTime",0.0,86_400.0) || !valid("duration",0.001,86_400.0)
                || !valid("sourceIn",0.0,86_400.0) || !valid("transitionDuration",0.0,86_400.0)
                || !valid("speed",0.1,8.0) || !valid("volume",0.0,1.0) || !clip["enabled"].is_boolean()
                || !matches!(clip["transitionIn"].as_str(),Some("none"|"fade"|"fade-black"|"slide-left"|"slide-right"|"zoom-in"|"zoom-out")) {
                return Err(invalid("时间线片段参数无效或 ID 重复"));
            }
            if clip["enabled"] == true || clip.get("assetId").is_some() {
                let id = timeline_asset(project,clip).ok_or_else(|| invalid(&format!("片段 {id} 缺少画面素材")))?;
                video_queue::media_path(app,file,id,"video").or_else(|_| video_queue::media_path(app,file,id,"image"))?;
            }
            if let Some(asset) = clip["voiceAssetId"].as_str() { video_queue::media_path(app,file,asset,"audio")?; }
        }
        project["timeline"] = json!(clips);
    }
    if let Some(text) = input.get("captionSrt") {
        project["captionSrt"] = json!(text.as_str().filter(|text| text.encode_utf16().count()<=50_000).ok_or_else(|| invalid("字幕文本无效"))?);
    }
    if let Some(bgm) = input.get("bgm") {
        if !bgm.is_null() { video_queue::media_path(app,file,required(bgm,"assetId")?,"audio")?; }
        project["bgmClip"] = bgm.clone();
    }
    if let Some(overlays) = input.get("overlays") {
        let overlays = overlays.as_array().filter(|items| items.len()<=250).ok_or_else(|| invalid("文字叠加列表无效"))?;
        project["textOverlays"] = json!(overlays);
    }
    let issues = final_export::timeline_issues(project);
    let warnings: Vec<_> = issues.iter().filter_map(|issue| issue["message"].as_str()).collect();
    Ok(json!({"clipCount":project["timeline"].as_array().map_or(0,Vec::len),"warnings":warnings,"timelineIssues":issues}))
}

pub(super) fn timeline_from_shots(project: &Value) -> Vec<Value> {
    timed_shots(project).filter_map(|(shot,start,end)| {
        let id = shot["id"].as_str()?;
        Some(json!({"id":format!("clip-{id}"),"shotId":id,"startTime":start,"duration":end-start,
            "sourceIn":0,"transitionIn":"none","transitionDuration":0,"speed":1,"volume":1,"enabled":true}))
    }).collect()
}

pub(super) fn resolve_shot_id(project: &Value, selector: &str) -> Result<String> {
    if project["shots"].as_array().into_iter().flatten().any(|shot| shot["id"] == selector) {
        return Ok(selector.to_owned());
    }
    let number = selector.strip_prefix('#').and_then(|text| text.parse::<usize>().ok())
        .and_then(|number| number.checked_sub(1))
        .ok_or_else(|| studio_mcp::missing("找不到指定镜头，请传镜头 ID 或项目镜号 #05"))?;
    let (shot,_,_) = timed_shots(project).nth(number).ok_or_else(|| studio_mcp::missing("镜号超出项目分镜范围"))?;
    Ok(required(shot,"id")?.to_owned())
}

pub(super) fn set_video_prompt(project: &mut Value, input: &Value) -> Result<Value> {
    let shot_id = resolve_shot_id(project,required(input,"shotId")?)?;
    let prompt = required(input,"prompt")?.trim();
    if prompt.chars().count() > 12000 { return Err(invalid("最终提示词无效")); }
    let shot = list(project,"shots")?.iter_mut().find(|shot| shot["id"] == shot_id)
        .ok_or_else(|| studio_mcp::missing("找不到指定镜头"))?;
    if shot["videoTaskId"].as_str().is_some_and(|id| !id.is_empty()) {
        return Err(invalid("该镜有从画布提交的视频任务，请先核对任务状态"));
    }
    shot["compiledVideoPrompt"] = json!(prompt);
    shot["compiledFromVideoPrompt"] = json!(shot["videoPrompt"].as_str().unwrap_or_default());
    Ok(json!({"shotId":shot_id,"warnings":shot_timing_warnings(shot)}))
}

fn update_shot_inputs(app: &App, file: &Path, project: &mut Value, input: &Value) -> Result<Value> {
    let shot_id = resolve_shot_id(project, required(input,"shotId")?)?;
    let shot_id = shot_id.as_str();
    let mut result = json!({});
    for (key, collection, label) in [("subjectIds", "subjects", "人物"), ("propIds", "props", "道具")] {
        let Some(ids) = input.get(key).filter(|ids| key != "propIds" || !ids.is_null()) else { continue };
        let ids = ids.as_array().filter(|items| items.len() <= 24).ok_or_else(|| invalid(&format!("{label}绑定格式无效")))?;
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            let id = id.as_str().ok_or_else(|| invalid(&format!("{label} ID 无效")))?;
            if !seen.insert(id) || !project[collection].as_array().into_iter().flatten().any(|item| item["id"] == id) {
                return Err(invalid(&format!("{label}绑定有重复或引用了不存在的{label}")));
            }
        }
    }
    if let Some(scene) = input.get("sceneId").filter(|value| !value.is_null()) {
        if !scene.as_str().is_some_and(|id| project["scenes"].as_array().into_iter().flatten().any(|item| item["id"] == id)) {
            return Err(invalid("场景绑定引用了不存在的场景"));
        }
    }
    for key in ["firstFrameAssetId","lastFrameAssetId"] {
        if let Some(value) = input.get(key).filter(|value| !value.is_null()) {
            let asset_id = value.as_str().ok_or_else(|| invalid("帧素材 ID 无效"))?;
            video_queue::media_path(app,file,asset_id,"image")?;
            generation_review::validate_frame(app,file,project,asset_id)?;
        }
    }
    let shot = list(project, "shots")?.iter_mut().find(|shot| shot["id"] == shot_id)
        .ok_or_else(|| invalid("找不到指定镜头"))?;
    let mut changed = false;
    if let Some(value) = input.get("videoProfileId") {
        if value.is_null() {
            shot.as_object_mut().ok_or_else(|| invalid("分镜格式无效"))?.remove("videoProfileId");
        } else {
            let id = value.as_str().filter(|id| !id.trim().is_empty()).ok_or_else(|| invalid("视频配置 ID 无效"))?;
            video_queue::get_profile(app, Some(id))?;
            shot["videoProfileId"] = value.clone();
        }
        changed = true;
    }
    if let Some(value) = input.get("duration") {
        if !value.as_f64().is_some_and(|seconds| seconds > 0.0 && seconds <= 120.0) {
            return Err(invalid("镜头时长必须大于 0 秒且不超过 120 秒"));
        }
        shot["duration"] = value.clone();
        changed = true;
    }
    for (key, allowed) in [
        ("audioMode", &["none","tts","h3-dialogue","h3-voiceover"][..]),
        ("subjectBindingMode", &["infer","explicit"][..]),
        ("h3GenMode", &["reference","fl2va"][..]),
    ] {
        if let Some(value) = input.get(key) {
            let value = value.as_str().filter(|mode| allowed.contains(mode)).ok_or_else(|| invalid("镜头输入模式无效"))?;
            shot[key] = json!(value);
            changed = true;
        }
    }
    if let Some(ids) = input.get("subjectIds") { shot["subjectIds"] = ids.clone(); changed = true; }
    if let Some(scene) = input.get("sceneId") { shot["sceneId"] = scene.clone(); changed = true; }
    if let Some(props) = input.get("propIds") {
        if props.is_null() { shot.as_object_mut().ok_or_else(|| invalid("分镜格式无效"))?.remove("propIds"); }
        else { shot["propIds"] = props.clone(); }
        changed = true;
    }
    for key in ["firstFrameAssetId","lastFrameAssetId"] {
        if let Some(value) = input.get(key) {
            if value.is_null() {
                shot.as_object_mut().ok_or_else(|| invalid("分镜格式无效"))?.remove(key);
            } else { shot[key] = value.clone(); }
            changed = true;
        }
    }
    let edited = shot.clone();
    if let Some(mode) = input.get("videoStillMode") {
        let mode = mode.as_str().filter(|mode| matches!(*mode, "identity" | "keyframe" | "storyboard"))
            .ok_or_else(|| invalid("配图策略无效"))?;
        let video = project["canvas"]["nodes"].as_array_mut().into_iter().flatten()
            .find(|node| node["type"] == "shotVideo" && node["data"]["shotId"] == shot_id)
            .ok_or_else(|| invalid("缺少分镜视频节点，请同步画布后再设置配图策略"))?;
        video["data"]["videoStillMode"] = json!(mode);
        changed = true;
    }
    if !changed { return Err(invalid("请至少提交一个输入设置")); }
    result["shotId"] = json!(shot_id);
    let node = video_queue::video_node(project, shot_id);
    result["shotInputs"] = shot_inputs(project, &edited, node);
    result["shot"] = edited.clone();
    result["warnings"] = json!(shot_timing_warnings(&edited));
    if result["shotInputs"]["canvasInputMode"] == "first-last-frame" {
        let selected = edited["videoProfileId"].as_str();
        let capability = video_queue::get_profile(app, selected).ok().and_then(|profile| video_queue::frame_capabilities(&profile));
        result["frameCapabilityCheck"] = match capability {
            Some((true,true)) => json!({"supportsFirst":true,"supportsLast":true}),
            Some((first,last)) => json!({"supportsFirst":first,"supportsLast":last,
                "warning":if first {"此工作流有首帧槽但没有尾帧槽，MCP 出片须选择首帧模式"} else {"此工作流没有首帧槽，画布首尾帧策略待换工作流验证"}}),
            None => json!({"warning":"尚无可验证的 API 工作流，已保存画布制作意图"}),
        };
    }
    Ok(result)
}

fn batch_edit_shots(app: &App, file: &Path, project: &mut Value, input: &Value) -> Result<Value> {
    let edits = input["edits"].as_array().filter(|edits| (1..=100).contains(&edits.len()))
        .ok_or_else(|| invalid("请提交 1 至 100 项分镜修改"))?;
    let mut items = Vec::with_capacity(edits.len());
    for (index, edit) in edits.iter().enumerate() {
        let mut apply = || -> Result<Value> {
            Ok(match required(edit,"kind")? {
                "video-prompt" => set_video_prompt(project,edit)?,
                "asset-prompt" => image_queue::save_prompt(project,edit)?,
                "script-prompt" => {
                    let id = resolve_shot_id(project,required(edit,"shotId")?)?;
                    let prompt = script_text(edit,"visualPrompt",12000)?;
                    let shot = project["shots"].as_array_mut().into_iter().flatten()
                        .find(|shot| shot["id"] == id).ok_or_else(|| invalid("找不到指定镜头"))?;
                    shot["visualPrompt"] = json!(prompt);
                    json!({"shotId":id,"visualPrompt":prompt})
                },
                "image-prompt" => {
                    let id = resolve_shot_id(project,required(edit,"shotId")?)?;
                    let mut args = edit.clone();
                    args["targetType"] = json!("shot-image");
                    args["targetId"] = json!(id);
                    image_queue::save_prompt(project,&args)?
                },
                "inputs" => update_shot_inputs(app,file,project,edit)?,
                "shot-fields" => set_shot_fields(project,edit)?,
                _ => return Err(invalid("未知分镜修改类型")),
            })
        };
        let result = apply().map_err(|mut error| {
            error.1 = format!("第 {} 项未应用：{}",index+1,error.1);
            error
        })?;
        items.push(result);
    }
    Ok(json!({"items":items}))
}

/// 更新镜头的编排类文本字段（不属于三种 prompt 的其余字段）。
fn set_shot_fields(project: &mut Value, edit: &Value) -> Result<Value> {
    let id = resolve_shot_id(project, required(edit, "shotId")?)?;
    let shot = project["shots"].as_array_mut().into_iter().flatten()
        .find(|shot| shot["id"] == id)
        .ok_or_else(|| invalid("找不到指定镜头"))?;
    let mut applied = serde_json::Map::new();
    for (key, value) in edit.as_object().into_iter().flatten() {
        let (limit, clear) = match key.as_str() {
            "kind" | "shotId" => continue,
            "title" | "cameraMovement" => (160, false),
            "audioDescription" => (4000, false),
            "shotSize" | "cutIn" => (160, true),
            "blocking" | "eyeline" | "continuity" | "purpose" | "propFocus" | "audienceGain" => (4000, true),
            "firstFramePrompt" | "lastFramePrompt" | "videoPrompt" => (12000, true),
            _ => return Err(invalid(&format!("不支持的分镜字段：{key}"))),
        };
        if value.is_null() && clear {
            shot.as_object_mut().ok_or_else(|| invalid("分镜格式无效"))?.remove(key);
        } else {
            let text = value.as_str().filter(|text| text.encode_utf16().count() <= limit)
                .ok_or_else(|| invalid(&format!("分镜字段 {key} 无效")))?;
            if (key == "title" && text.trim().is_empty()) ||
                (key == "cutIn" && !matches!(text, "action-start" | "mid-action" | "after-action" | "on-reaction")) ||
                (key == "cameraMovement" && !matches!(text, "static" | "push" | "pull" | "pan" | "tilt" | "orbit")) {
                return Err(invalid(&format!("分镜字段 {key} 无效")));
            }
            shot[key] = value.clone();
        }
        applied.insert(key.to_owned(), value.clone());
    }
    if applied.is_empty() {
        return Err(invalid("没有可更新的字段"));
    }
    Ok(json!({"shotId": id, "applied": applied}))
}

pub(super) fn invoke(app: &App, name: &str, input: &Value, active: Option<&(String,String)>) -> Result<Value> {
    if matches!(name,"studio_set_video_prompt" | "studio_update_shot_inputs" | "studio_set_asset_prompt") && input.get("items").is_some() {
        if input.as_object().is_none_or(|fields| fields.keys().any(|key| !matches!(key.as_str(),"projectId"|"expectedUpdatedAt"|"items"))) {
            return Err(invalid("单项字段和 items 批量模式只能选一种"));
        }
        let items = input["items"].as_array().filter(|items| (1..=100).contains(&items.len()))
            .ok_or_else(|| invalid("请提交 1 至 100 项修改"))?;
        let kind = match name {
            "studio_set_video_prompt" => "video-prompt",
            "studio_set_asset_prompt" => "asset-prompt",
            _ => "inputs",
        };
        let edits: Vec<_> = items.iter().map(|item| {
            let mut entry = item.as_object().cloned().ok_or_else(|| invalid("批量项须为对象"))?;
            entry.insert("kind".into(),json!(kind));
            Ok(Value::Object(entry))
        }).collect::<Result<_>>()?;
        return invoke(app,"studio_batch_edit_shots",&json!({
            "projectId":input["projectId"],"expectedUpdatedAt":input["expectedUpdatedAt"],"edits":edits
        }),active);
    }
    if name == "studio_create_project" { return create(app, input); }
    let id = required(input, "projectId")?;
    let (file, mut project) = studio_mcp::active_project(app)?.filter(|(_, p)| p["id"] == id)
        .ok_or_else(|| invalid("请先打开指定项目"))?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    if name == "studio_timeline" {
        let entries = project["timeline"].as_array().map_or(&[][..], Vec::as_slice);
        let offset = input["offset"].as_u64().unwrap_or(0).min(entries.len() as u64) as usize;
        let limit = input["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
        let end = (offset + limit).min(entries.len());
        return Ok(json!({ "projectId": id, "updatedAt": revision, "items": &entries[offset..end],
            "total": entries.len(), "nextOffset": (end < entries.len()).then_some(end),
            "captionSrt":project["captionSrt"],"bgm":project["bgmClip"],"overlays":project["textOverlays"],
            "finalVideoAssetId":project["finalVideoAssetId"],"finalVideoStale":final_export::stale(&project),
            "timelineIssues":final_export::timeline_issues(&project) }));
    }
    if name == "studio_project_settings" {
        let shot_id = input["shotId"].as_str();
        let entity = if let Some(id) = shot_id {
            Some(project["shots"].as_array().into_iter().flatten().find(|shot| shot["id"] == id)
                .ok_or_else(|| invalid("找不到指定镜头"))?)
        } else { None };
        let node = shot_id.and_then(|id| video_queue::video_node(&project, id));
        let profile_id = entity.and_then(|shot| shot["videoProfileId"].as_str());
        let effective = video_queue::get_profile(app, profile_id).ok().map(|profile| json!({
            "id": profile["id"], "name": profile["name"], "adapter": profile["adapter"]["type"], "workflowId": profile["workflowId"]
        }));
        return Ok(json!({ "projectId": id, "updatedAt": revision, "folderPath": file.parent(), "root": app.root,
            "projectAspectRatio": project["settings"]["aspectRatio"], "defaultVideoResolution": project["settings"]["defaultVideoResolution"],
            "effectiveAspectRatio": video_queue::aspect_of(&project, shot_id.unwrap_or(""))?, "effectiveVideo": effective,
            "projectFrameChainMode": project["settings"]["h3ShotChainMode"].as_str().unwrap_or("auto"),
            "shotInputs": entity.map(|shot| shot_inputs(&project, shot, node)) }));
    }
    // Confirmed changes to shot order must use the revision the user reviewed.
    validate_project_revision(input, revision, !(changes_shot_order(name) && input["confirmed"] == true))?;
    let mut result = json!({});
    if changes_shot_order(name) {
        let context = video_queue::shot_change_context(app, id, active)?;
        result = if name == "studio_replace_shots" { replace_shots(&mut project, input, &context)? }
            else { shot_change(&mut project, name, input, &context)? };
        if result["applied"] != true {
            result["projectId"] = json!(id);
            result["updatedAt"] = json!(revision);
            return Ok(result);
        }
    } else { match name {
        "studio_import_script" => { import(&mut project, input)?; }
        "studio_rename_project" => {
            let name = required(input, "name")?.trim();
            if name.encode_utf16().count() > 100 { return Err(invalid("项目名过长")); }
            project["name"] = json!(name);
            result["name"] = json!(name);
        }
        "studio_update_script" => {
            if !project["script"].is_object() { return Err(invalid("尚无剧本")); }
            if !["content", "topic", "keywords", "style"].iter().any(|key| input.get(key).is_some()) {
                return Err(invalid("请提供要更新的剧本字段"));
            }
            if input.get("content").is_some() {
                let content = script_text(input, "content", 50_000)?;
                if project["script"]["content"].as_str() != Some(content) {
                    project["script"]["content"] = json!(content);
                    project["script"]["playReady"] = json!(true);
                    project["playApproved"] = json!(false);
                }
            }
            if input.get("topic").is_some() {
                project["script"]["topic"] = json!(script_text(input, "topic", 160)?.trim());
            }
            if input.get("style").is_some() {
                let style = required(input, "style")?;
                if !matches!(style, "cinematic" | "anime" | "oil-painting" | "3d-render" |
                    "photorealistic" | "watercolor" | "pixel-art" | "ink-wash") {
                    return Err(invalid("剧本风格无效"));
                }
                project["script"]["style"] = json!(style);
            }
            if let Some(keywords) = input.get("keywords") {
                let items = keywords.as_array().filter(|items| items.len() <= 50)
                    .ok_or_else(|| invalid("关键词必须是最多 50 项的列表"))?;
                let words = items.iter().map(|item| item.as_str().map(str::trim)
                    .filter(|word| !word.is_empty() && word.encode_utf16().count() <= 160)
                    .ok_or_else(|| invalid("关键词无效"))).collect::<Result<Vec<_>>>()?;
                project["script"]["keywords"] = json!(words);
            }
            result["topic"] = project["script"]["topic"].clone();
            result["playApproved"] = json!(project["playApproved"] == true);
        }
        "studio_approve_script" => {
            if !project["script"].is_object() { return Err(invalid("尚无剧本")); }
            let approved = input["approved"].as_bool().unwrap_or(true);
            if approved && project["script"]["playReady"] != true {
                return Err(invalid("剧本还没有可用的正文，无法确认"));
            }
            project["playApproved"] = json!(approved);
            result["playApproved"] = json!(approved);
        }
        "studio_upsert_project_assets" => asset_cards(&mut project, input)?,
        "studio_set_asset_prompt" => result = image_queue::save_prompt(&mut project,input)?,
        "studio_append_shots" => result = append(&mut project, input)?,
        "studio_edit_timeline" => result = edit_timeline(app,&file,&mut project,input)?,
        "studio_update_project_defaults" => {
            if let Some(mode) = input.get("h3ShotChainMode") {
                let mode = mode.as_str().filter(|mode| matches!(*mode, "auto" | "fl2va"))
                    .ok_or_else(|| invalid("项目首尾帧链模式无效"))?;
                project["settings"]["h3ShotChainMode"] = json!(mode);
                result["projectFrameChainMode"] = json!(mode);
            }
            if let Some(aspect) = input["aspectRatio"].as_str() {
                let (width, height) = match aspect {
                    "16:9" => (1280, 720), "9:16" => (720, 1280), "1:1" => (1024, 1024),
                    "4:3" => (1152, 864), "21:9" => (1344, 576),
                    _ => return Err(invalid("画幅无效")),
                };
                project["settings"]["aspectRatio"] = json!(aspect);
                project["settings"]["width"] = json!(width);
                project["settings"]["height"] = json!(height);
                if project["script"].is_object() { project["script"]["aspectRatio"] = json!(aspect); }
            }
            if let Some(resolution) = input["defaultVideoResolution"].as_str() {
                if !["768p", "1080p"].contains(&resolution) { return Err(invalid("分辨率无效")); }
                project["settings"]["defaultVideoResolution"] = json!(resolution);
            }
        }
        "studio_update_shot_inputs" => result = update_shot_inputs(app,&file,&mut project,input)?,
        "studio_batch_edit_shots" => result = batch_edit_shots(app,&file,&mut project,input)?,
        "studio_revise_shot" | "studio_bind_shot_asset" => {
            let shot_id = resolve_shot_id(&project,required(input,"shotId")?)?;
            if name == "studio_bind_shot_asset" && matches!(input["slot"].as_str(),Some("first"|"last")) {
                let asset_id = required(input,"assetId")?;
                video_queue::media_path(app,&file,asset_id,"image")?;
                generation_review::validate_frame(app,&file,&project,asset_id)?;
            }
            let shot = list(&mut project, "shots")?.iter_mut().find(|shot| shot["id"] == shot_id)
                .ok_or_else(|| invalid("找不到指定镜头"))?;
            if name == "studio_revise_shot" {
                if let Some(prompt) = input["visualPrompt"].as_str() { shot["visualPrompt"] = json!(prompt); }
                if let Some(text) = input["voiceover"].as_str() { shot["voiceover"] = json!(text); }
                result["warnings"] = json!(shot_timing_warnings(shot));
            } else {
                let asset_id = required(input, "assetId")?;
                let slot = required(input, "slot")?;
                let (key, history_key, kind) = match slot {
                    "board" => ("boardAssetId", Some("boardHistoryIds"), "image"),
                    "image" => ("imageAssetId", Some("imageHistoryIds"), "image"),
                    "video" => ("videoAssetId", Some("videoHistoryIds"), "video"),
                    "audio" => ("audioAssetId", Some("audioHistoryIds"), "audio"),
                    "first" => ("firstFrameAssetId", None, "image"),
                    "last" => ("lastFrameAssetId", None, "image"),
                    _ => return Err(invalid("素材位置无效")),
                };
                let folder = file.parent().ok_or_else(|| invalid("项目目录无效"))?;
                let path = scoped_path(&app.root, &folder.join("assets/manifest.json"))?;
                let manifest: Vec<Value> = serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid("素材清单无效"))?;
                if !manifest.iter().any(|asset| asset["id"] == asset_id && asset["kind"] == kind) {
                    return Err(invalid("指定素材不存在或类型不符"));
                }
                let old = shot[key].as_str().map(str::to_owned);
                if let Some(history_key) = history_key {
                    let history = list(shot, history_key)?;
                    history.retain(|value| value != asset_id);
                    if let Some(old) = old.as_ref().filter(|old| *old != asset_id) {
                        if !history.contains(&json!(old)) { history.insert(0, json!(old)); }
                    }
                }
                shot[key] = json!(asset_id);
                if slot == "audio" && shot["audioMode"].is_null() {
                    shot["audioMode"] = json!("tts");
                }
                result["bind"] = json!({"slot":slot,"applied":true,"previous":old,"current":asset_id});
                match slot {
                    "image" => {
                        result["bind"]["affectsVideo"] = json!(false);
                        result["bind"]["note"] = json!("已更新分镜主图。首帧模式读取 first 槽或本次显式输入。若要让视频首帧使用同一张图，请同时绑定 first。");
                    }
                    "first" | "last" => {
                        result["bind"]["affectsVideo"] = json!(true);
                        result["bind"]["note"] = json!("已更新对应帧槽。后续预检仅在实际生成模式使用该帧且没有显式覆盖时采用。");
                    }
                    _ => {}
                }
                result["imageConsistency"] = json!(image_consistency(shot));
            }
            result["shotId"] = json!(shot_id);
        }
        _ => return Err(invalid("未知项目操作")),
    } }
    if changes_shot_order(name) || name == "studio_append_shots" ||
        (name == "studio_update_shot_inputs" && input.get("duration").is_some()) ||
        (name == "studio_batch_edit_shots" && input["edits"].as_array().into_iter().flatten().any(|edit| edit["kind"] == "inputs" && edit.get("duration").is_some())) {
        refresh_timed_titles(&mut project);
        let refresh_title = |entry: &mut Value| {
            if entry["shot"].is_object() {
                if let Some(saved) = project["shots"].as_array().into_iter().flatten().find(|shot| shot["id"] == entry["shotId"]) {
                    entry["shot"]["title"] = saved["title"].clone();
                }
            }
        };
        refresh_title(&mut result);
        for entry in result.get_mut("items").and_then(Value::as_array_mut).into_iter().flatten() { refresh_title(entry); }
    }
    sync_shot_node_labels(&mut project);
    let revision = now().max(revision.saturating_add(1));
    project["updatedAt"] = json!(revision);
    save_project(&file, &project)?;
    notify_project_revision(app, json!({"projectId": id, "updatedAt": revision}));
    result["projectId"] = json!(id);
    result["updatedAt"] = json!(revision);
    Ok(result)
}
