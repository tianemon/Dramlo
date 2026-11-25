use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::rt::TokioIo;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_RESPONSE: usize = 256 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    project_id: String,
    request_id: String,
    revision: u64,
    status: String,
    pause_requested: bool,
    #[serde(default)]
    stop_wait_requested: bool,
    #[serde(default)]
    cancelled_prompt_id: Option<String>,
    steps: Vec<Step>,
    error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    shot_id: String,
    title: Value,
    prompt: String,
    profile_id: String,
    profile_stamp: u64,
    shot_stamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    binding_stamp: Option<u64>,
    aspect: String,
    previous_shot_id: Option<String>,
    previous_asset_id: Option<String>,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_id: Option<String>,
    asset_id: String,
    take: Value,
    #[serde(default)]
    template_images: Vec<Value>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    reference_assignments: Vec<Value>,
    #[serde(default)]
    media_inputs: Vec<Value>,
    #[serde(default)]
    reference_facts: Value,
    #[serde(default)]
    workload: Value,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bind: Option<VideoBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extension_state: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoBinding {
    applied: bool,
    reason: String,
}

use serde::Serialize;

pub(super) const METHODS: &[&str] = &[
    "studio_check_video_readiness", "studio_prepare_video", "studio_confirm_video", "studio_video_status",
    "studio_current_video_batch", "studio_pause_video", "studio_resume_video", "studio_discard_video_plan",
    "studio_stop_video_wait", "studio_cancel_video_shot", "studio_recover_video_id",
];

pub(super) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn job_path(app: &App, project: &str, request: &str) -> Result<PathBuf> {
    kv_path(app, &format!("video-batch:{}", json!([project, request])))
}

fn write_batch(app: &App, batch: &Batch) -> Result<()> {
    let path = job_path(app, &batch.project_id, &batch.request_id)?;
    atomic_save(&path, &serde_json::to_vec(batch).map_err(|_| invalid("无法保存视频批次"))?)
}

fn read_batch(app: &App, project: &str, request: &str) -> Result<Batch> {
    let path = job_path(app, project, request)?;
    serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid("视频批次格式无效"))
}

fn response(app: &App, batch: &Batch) -> Value {
    let mut value = json!(batch);
    let project = batch.steps.iter().any(|step| step.status == "saved")
        .then(|| project_file(app,&batch.project_id).ok()).flatten();
    if let Some(steps) = value["steps"].as_array_mut() {
        for (step, saved) in steps.iter_mut().zip(&batch.steps) {
            if let Some(object) = step.as_object_mut() {
                object.remove("profileStamp");
                object.remove("shotStamp");
                object.remove("bindingStamp");
                if batch.status == "discarded" { object.insert("status".into(),json!("discarded")); }
                if let Some(bind) = saved.bind.as_ref().filter(|_| saved.status == "saved") {
                    let current = project.as_ref().and_then(|(_, project)| shot(project,&saved.shot_id).ok())
                        .and_then(|shot| shot["videoAssetId"].as_str());
                    object.insert("bind".into(), json!({"slot":"video","applied":bind.applied,
                        "reason":bind.reason,
                        "current":current,"isCurrent":current == Some(saved.asset_id.as_str())}));
                }
            }
        }
    }
    value["workload"] = generation_review::sum_workload(batch.steps.iter().map(|step| &step.workload));
    value
}

fn project_file(app: &App, id: &str) -> Result<(PathBuf, Value)> {
    let (file, project) = studio_mcp::project_by_id(app, id)?;
    let projected = capture_project(&project)?.into_owned();
    Ok((file, projected))
}

// Use the same ComfyUI queue for shot records and persisted continuous-capture
// nodes. The projection is used for preflight/execution only and never saved.
fn capture_project(project: &Value) -> Result<std::borrow::Cow<'_, Value>> {
    let capture = &project["continuousCapture"];
    let Some(capture_id) = capture["id"].as_str().filter(|_| project["continuousCaptureEnabled"] == true) else {
        return Ok(std::borrow::Cow::Borrowed(project));
    };
    if !capture_plan::matches_script(project) {
        return Ok(std::borrow::Cow::Borrowed(project));
    }
    let mut shots = project["shots"].as_array().cloned().unwrap_or_default();
    let mut index = shots.iter().filter_map(|shot| shot["index"].as_u64()).max().unwrap_or(0);
    let nodes = project["canvas"]["nodes"].as_array().map(Vec::as_slice).unwrap_or_default();
    for scene in capture["plan"]["scenes"].as_array().into_iter().flatten() {
        let Some(scene_id) = scene["id"].as_str() else { continue };
        for (at, _) in scene["segments"].as_array().into_iter().flatten().enumerate() {
            let Some(id) = capture["videoNodes"][format!("{scene_id}:{at}")].as_str() else { continue };
            let Some(node) = nodes.iter().find(|node| node["id"] == id && node["type"] == "videoGen"
                && node["data"]["captureId"] == capture_id) else { continue };
            if shots.iter().any(|shot| shot["id"] == id) {
                return Err(invalid("连续拍节点与分镜 ID 重复"));
            }
            index = index.checked_add(1).ok_or_else(|| invalid("分镜顺序无效"))?;
            let data = &node["data"];
            let opening_id = &capture["openingNodes"][scene_id];
            let first = nodes.iter().find(|node| node["id"] == *opening_id && node["type"] == "imageGen")
                .map(|node| &node["data"]["outputAssetId"]);
            shots.push(json!({
                "id":id,"index":index,"sceneId":scene_id,"chapterId":capture_id,"captureNode":true,
                "title":data["label"],"compiledVideoPrompt":data["prompt"],"duration":data["duration"],
                "videoProfileId":data["videoProfileId"],"videoAssetId":data["outputAssetId"],
                "firstFrameAssetId":if at == 0 { data.get("firstFrameAssetId").filter(|value| value.is_string()).or(first) } else { None },
                "referenceAssetIds":data["referenceAssetIds"],
                "canvasSubmissionPending":data["captureSubmissionPending"] == true || data["videoTaskId"].as_str().is_some_and(|id| !id.is_empty())
            }));
        }
    }
    let mut projected = project.clone();
    projected["shots"] = json!(shots);
    Ok(std::borrow::Cow::Owned(projected))
}

pub(super) fn get_profile(app: &App, id: Option<&str>) -> Result<Value> {
    let record: Value = serde_json::from_slice(&fs::read(kv_path(app, "settings")?)?)
        .map_err(|_| invalid("设置文件无效"))?;
    let settings: Value = serde_json::from_str(record["value"].as_str().unwrap_or_default())
        .map_err(|_| invalid("工作台模型设置无效"))?;
    let id = id.or_else(|| settings["activeVideoId"].as_str()).ok_or_else(|| invalid("请选择视频接入"))?;
    settings["videos"].as_array().and_then(|items| items.iter().find(|item| item["id"].as_str() == Some(id)))
        .cloned().ok_or_else(|| invalid("找不到视频接入"))
}

pub(super) fn stamp(value: &Value) -> u64 {
    value.to_string().bytes().fold(0xcbf29ce484222325u64, |hash, value| {
        (hash ^ u64::from(value)).wrapping_mul(0x100000001b3)
    })
}

pub(super) fn profile_stamp(profile: &Value) -> u64 {
    stamp(&json!([profile["baseUrl"], profile["adapter"], profile["workflowId"]]))
}

fn shot_stamp(entity: &Value) -> u64 {
    stamp(&json!([entity["compiledVideoPrompt"], entity["duration"], entity["videoProfileId"],
        entity["referenceAssetIds"], entity["firstFrameAssetId"], entity["lastFrameAssetId"],
        entity["sceneId"], entity["chapterId"], entity["index"]]))
}

fn binding_stamp(shot: &Value) -> u64 {
    stamp(&json!([shot.get("subjectBindingMode"), shot.get("subjectIds"), shot.get("propIds")]))
}

impl Step {
    fn matches_shot(&self, shot: &Value) -> bool {
        shot_stamp(shot) == self.shot_stamp &&
            self.binding_stamp.is_none_or(|saved| saved == binding_stamp(shot))
    }
}

pub(super) fn endpoint(profile: &Value) -> Result<SocketAddr> {
    if profile["adapter"]["type"] != "comfyui-prompt" {
        return Err(invalid("Rust 队列需要原生 ComfyUI API 接入"));
    }
    let address = profile["baseUrl"].as_str().unwrap_or_default().trim_end_matches('/');
    let port = address.strip_prefix("http://127.0.0.1:").or_else(|| address.strip_prefix("http://localhost:"))
        .and_then(|port| port.parse::<u16>().ok()).filter(|port| *port != 0)
        .ok_or_else(|| invalid("请使用本机 ComfyUI HTTP 根地址，例如 http://127.0.0.1:8188"))?;
    Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
}

pub(super) fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| invalid("视频计划缺少必要字段"))
}

fn shot<'a>(project: &'a Value, id: &str) -> Result<&'a Value> {
    project["shots"].as_array().and_then(|shots| shots.iter().find(|shot| shot["id"].as_str() == Some(id)))
        .ok_or_else(|| invalid("找不到分镜"))
}

pub(super) fn media_path(app: &App, project: &Path, id: &str, kind: &str) -> Result<PathBuf> {
    let folder = project.parent().ok_or_else(|| invalid("项目目录无效"))?;
    let file = scoped_path(&app.root, &folder.join("assets/manifest.json"))?;
    let manifest: Value = serde_json::from_slice(&fs::read(file)?).map_err(|_| invalid("素材清单无效"))?;
    let item = manifest.as_array().and_then(|items| items.iter().find(|item| item["id"].as_str() == Some(id) && item["kind"] == kind))
        .ok_or_else(|| invalid("找不到项目图片素材"))?;
    let path = folder.join(field(item, "fileRelPath")?);
    if !path.starts_with(folder) {
        return Err(invalid("图片不在当前项目目录"));
    }
    let path = scoped_path(&app.root, &path)?;
    if !path.is_file() {
        return Err(invalid("图片原文件缺失"));
    }
    Ok(path)
}

fn previous_shot<'a>(project: &'a Value, target: &Value) -> Result<&'a Value> {
    let index = target["index"].as_u64().ok_or_else(|| invalid("镜头顺序无效"))?;
    let previous = project["shots"].as_array().into_iter().flatten()
        .filter(|shot| shot["index"].as_u64().is_some_and(|other| other < index))
        .max_by_key(|shot| shot["index"].as_u64()).ok_or_else(|| invalid("找不到上一镜"))?;
    if !target["sceneId"].is_string() || target["sceneId"] != previous["sceneId"] ||
        target["chapterId"] != previous["chapterId"] {
        return Err(invalid("跨场或跨章节镜头不能继承尾帧"));
    }
    Ok(previous)
}

pub(super) fn video_node<'a>(project: &'a Value, shot: &str) -> Option<&'a Value> {
    project["canvas"]["nodes"].as_array().into_iter().flatten()
        .find(|node| (node["type"] == "shotVideo" && node["data"]["shotId"] == shot) ||
            (node["type"] == "videoGen" && node["id"] == shot && node["data"]["captureId"].is_string()))
}

fn chosen_profile<'a>(shot: &'a Value, take: &'a Value) -> Option<&'a str> {
    take["profileId"].as_str().or_else(|| shot["videoProfileId"].as_str())
}

pub(super) fn aspect_of(project: &Value, shot: &str) -> Result<&'static str> {
    let aspect = video_node(project, shot)
        .and_then(|node| node["data"]["aspectRatio"].as_str())
        .or_else(|| project["settings"]["aspectRatio"].as_str()).unwrap_or("16:9");
    match aspect {
        "16:9" => Ok("16:9"), "9:16" => Ok("9:16"), "1:1" => Ok("1:1"),
        "4:3" => Ok("4:3"), "21:9" => Ok("21:9"),
        _ => Err(invalid("镜头画幅无效")),
    }
}

pub(super) fn bind(graph: &mut Value, binding: &Value, value: Value) -> Result<()> {
    let node = field(binding, "nodeId")?;
    let input = field(binding, "field")?;
    let target = graph.get_mut(node).and_then(|node| node.get_mut("inputs"))
        .and_then(|inputs| inputs.get_mut(input))
        .filter(|value| value.is_string() || value.is_number()).ok_or_else(|| invalid("工作流输入映射已失效"))?;
    *target = value;
    Ok(())
}

fn source_images(graph: &Value, link: &Value) -> Vec<String> {
    let mut pending = vec![link];
    let mut seen = std::collections::HashSet::new();
    let mut output = Vec::new();
    while let Some(link) = pending.pop() {
        let Some(id) = link.as_array().filter(|items| items.len() == 2).and_then(|items| items[0].as_str()) else { continue; };
        if !seen.insert(id) { continue; }
        let node = &graph[id];
        let kind = node["class_type"].as_str().unwrap_or_default().to_ascii_lowercase();
        if kind.contains("loadimage") && node["inputs"]["image"].is_string() {
            output.push(id.to_owned());
        } else if let Some(inputs) = node["inputs"].as_object() {
            pending.extend(inputs.values().filter(|value| value.is_array()));
        }
    }
    output
}

fn first_link(node: &Value) -> &Value {
    let inputs = &node["inputs"];
    if inputs["first_frame"].is_array() { return &inputs["first_frame"]; }
    if inputs["task_type"] == "I2VA" && node["class_type"].as_str().is_some_and(|kind| kind.starts_with("MiniMaxH3")) { return &inputs["ref_images.ref_image_0"]; }
    &Value::Null
}

pub(super) fn inferred_bindings(graph: &Value) -> Vec<Value> {
    let Some(nodes) = graph.as_object() else { return Vec::new(); };
    let mut result = Vec::new();
    let mut refs = Vec::new();
    let mut first = Vec::new();
    let mut last = Vec::new();
    for (id, node) in nodes {
        let class = node["class_type"].as_str().unwrap_or_default();
        let title = node["_meta"]["title"].as_str().unwrap_or_default().to_lowercase();
        let inputs = &node["inputs"];
        if (class == "CR Prompt Text" || class.starts_with("MiniMaxH3")) && inputs["prompt"].is_string() {
            result.push(json!({ "role": "prompt", "nodeId": id, "field": "prompt" }));
        }
        if (title.contains("duration") || title.contains("时长") || title.contains("秒数")) && inputs["value"].is_number() {
            result.push(json!({ "role": "length", "nodeId": id, "field": "value", "durationUnit": "seconds" }));
        }
        first.extend(source_images(graph, first_link(node)));
        last.extend(source_images(graph, &inputs["last_frame"]));
        last.extend(source_images(graph, &inputs["end_frame"]));
        if let Some(fields) = inputs.as_object() {
            for (name, link) in fields {
                let Some(order) = name.strip_prefix("ref_images.ref_image_").and_then(|value| value.parse::<usize>().ok()) else { continue; };
                let candidates = source_images(graph, link);
                if candidates.len() == 1 { refs.push((order, candidates[0].clone())); }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    refs.sort_by_key(|(index, _)| *index);
    for node in first.into_iter().chain(refs.into_iter().map(|(_, node)| node)) {
        if seen.insert(node.clone()) { result.push(json!({ "role": "image", "nodeId": node, "field": "image" })); }
    }
    last.sort();
    last.dedup();
    if last.len() == 1 { result.push(json!({ "role": "imageEnd", "nodeId": last[0], "field": "image" })); }
    result
}

struct Prepared {
    graph: Value,
    media: Vec<(Value, PathBuf)>,
    continuation: Option<Value>,
    template_images: Vec<Value>,
    warnings: Vec<String>,
    reference_assignments: Vec<Value>,
    media_inputs: Vec<Value>,
    reference_facts: Value,
    workload: Value,
    output: Option<Value>,
}

pub(super) fn frame_capabilities(profile: &Value) -> Option<(bool, bool)> {
    let raw: Value = serde_json::from_str(profile["adapter"]["workflowTemplate"].as_str()?).ok()?;
    let graph = raw.get("prompt").unwrap_or(&raw);
    let nodes = graph.as_object().filter(|nodes| !nodes.is_empty() &&
        nodes.values().all(|node| node["class_type"].is_string() && node["inputs"].is_object()))?;
    Some((nodes.values().any(|node| first_link(node).is_array()),
        nodes.values().any(|node| node["inputs"]["last_frame"].is_array() || node["inputs"]["end_frame"].is_array())))
}

pub(super) fn workflow_template(profile: &Value) -> Result<Value> {
    let raw: Value = serde_json::from_str(field(&profile["adapter"], "workflowTemplate")?)
        .map_err(|_| invalid("无效的工作流 JSON"))?;
    let graph = raw.get("prompt").unwrap_or(&raw);
    if !graph.as_object().is_some_and(|nodes| !nodes.is_empty() && nodes.values().all(|node| node["class_type"].is_string() && node["inputs"].is_object())) {
        return Err(invalid("请导入 ComfyUI API 格式工作流"));
    }
    Ok(graph.clone())
}

pub(super) fn workflow_bindings(profile: &Value, graph: &Value) -> Vec<Value> {
    let mut bindings = profile["adapter"]["inputMap"]["bindings"].as_array().cloned().unwrap_or_default();
    let guessed = inferred_bindings(graph);
    for entry in &guessed {
        let role = &entry["role"];
        if role == "image" {
            if !bindings.iter().any(|binding| binding["nodeId"] == entry["nodeId"] && binding["field"] == entry["field"]) {
                bindings.push(entry.clone());
            }
        } else if !bindings.iter().any(|binding| binding["role"] == *role) &&
            guessed.iter().filter(|binding| binding["role"] == *role).count() == 1 {
            bindings.push(entry.clone());
        }
    }
    bindings
}

fn frame_sources(graph: &Value) -> (std::collections::HashSet<String>,std::collections::HashSet<String>) {
    let nodes = graph.as_object().into_iter().flat_map(|nodes| nodes.values());
    let first_sources: std::collections::HashSet<_> = nodes.clone().flat_map(|node| source_images(graph, first_link(node))).collect();
    let last_sources: std::collections::HashSet<_> = nodes.clone().flat_map(|node| {
        source_images(graph, &node["inputs"]["last_frame"]).into_iter().chain(source_images(graph, &node["inputs"]["end_frame"]))
    }).collect();
    (first_sources,last_sources)
}

fn matches_frame(slot: &Value, sources: &std::collections::HashSet<String>) -> bool {
    slot["field"] == "image" && slot["nodeId"].as_str().is_some_and(|id| sources.contains(id))
}

fn configured_bindings(profile: &Value, graph: &Value) -> Result<Vec<Value>> {
    let mut bindings = workflow_bindings(profile,graph);
    for binding in comfy_config::explicit_bindings(profile) {
        if !bindings.iter().any(|item| item["role"] == "image" && item["nodeId"] == binding["nodeId"] && item["field"] == "image") {
            bindings.push(binding);
        }
    }
    if let Some(binding) = comfy_config::length_binding(profile,graph,&bindings)? {
        bindings.retain(|item| item["role"] != "length");
        bindings.push(binding);
    }
    Ok(bindings)
}

fn aspect_for_take<'a>(project: &'a Value, shot: &str, take: &'a Value, profile: &'a Value) -> Result<&'a str> {
    if take.get("aspectRatio").is_none() && !profile["adapter"]["outputConfig"].is_object() {
        return aspect_of(project,shot);
    }
    let fallback = video_node(project,shot).and_then(|node| node["data"]["aspectRatio"].as_str())
        .or_else(|| project["settings"]["aspectRatio"].as_str());
    comfy_config::aspect(profile,fallback,take.get("aspectRatio"))
}

pub(super) fn describe_workflow(profile: &Value) -> Result<Value> {
    let mut graph = workflow_template(profile)?;
    let bindings = configured_bindings(profile, &graph)?;
    let mut targets = std::collections::HashSet::new();
    let mut prompt = false;
    let mut length = false;
    for binding in &bindings {
        let value = match binding["role"].as_str() {
            Some("prompt") => { prompt = true; json!("预检") }
            Some("length") if binding["durationUnit"] == "seconds" => { length = true; json!(4) }
            Some("width" | "height") => json!(1024),
            Some("image" | "imageEnd") => json!("预检"),
            _ => continue,
        };
        let target = (field(binding, "nodeId")?, field(binding, "field")?);
        if !targets.insert(target) { return Err(invalid("同一个工作流输入不能绑定多个角色或重复使用")); }
        bind(&mut graph, binding, value)?;
    }
    let (first_sources,last_sources) = frame_sources(&graph);
    let slot_info = |binding: &Value| {
        let id = binding["nodeId"].as_str().unwrap_or_default();
        let node = &graph[id];
        let mut consumers = Vec::new();
        let mut picture_numbers = std::collections::BTreeSet::new();
        for (consumer_id,consumer) in graph.as_object().into_iter().flatten() {
            for (field,link) in consumer["inputs"].as_object().into_iter().flatten() {
                let picture = field.strip_prefix("ref_images.ref_image_").and_then(|index|index.parse::<u64>().ok())
                    .and_then(|index| index.checked_add(1));
                if picture.is_none() && !matches!(field.as_str(),"first_frame"|"last_frame"|"end_frame") { continue; }
                if !source_images(&graph,link).iter().any(|source| source == id) { continue; }
                consumers.push(json!({"nodeId":consumer_id,"field":field}));
                picture_numbers.extend(picture);
            }
        }
        let picture = if picture_numbers.len() == 1 { picture_numbers.first().copied() } else { None };
        json!({"nodeId":binding["nodeId"],"field":binding["field"],"label":node["_meta"]["title"],
            "classType":node["class_type"],"mediaMode":binding["mediaMode"].as_str().unwrap_or("upload"),
            "consumers":consumers,"pictureNumber":picture})
    };
    let refs: Vec<_> = bindings.iter().filter(|slot| slot["role"] == "image" &&
        !matches_frame(slot,&first_sources) && !matches_frame(slot,&last_sources)).collect();
    let declared = comfy_config::reference_slots(profile,&refs)?;
    let mut reference_slots: Vec<_> = refs.iter().enumerate().map(|(index,binding)| {
        let mut item = slot_info(binding);
        item["index"] = json!(index);
        item
    }).collect();
    if let Some(slots) = &declared {
        reference_slots = slots.iter().map(|slot| {
            let binding = refs.iter().find(|binding| binding["nodeId"] == slot["nodeId"]).unwrap();
            let mut item = slot_info(binding);
            for key in ["index","role","required","fallback"] {
                if let Some(value) = slot.get(key) { item[key] = value.clone(); }
            }
            item
        }).collect();
    }
    let min = declared.as_ref().map_or(refs.len(),|slots| slots.iter().filter(|slot| slot["required"] == true).count());
    let mode = if declared.is_some() { "ordered-optional" } else { "fixed-ordered" };
    let first_slot = bindings.iter().find(|slot| slot["role"] == "image" && matches_frame(slot,&first_sources)).map(slot_info);
    let last_slot = bindings.iter().find(|slot| slot["role"] == "imageEnd" && matches_frame(slot,&last_sources)).map(slot_info);
    let (first, last) = frame_capabilities(profile).unwrap_or_default();
    let missing: Vec<_> = [("prompt", prompt), ("length", length)].into_iter()
        .filter_map(|(role, present)| (!present).then_some(role)).collect();
    Ok(json!({"ready":missing.is_empty(),"missingMapping":missing,"hasFirstFrame":first,"hasLastFrame":last,
        "referenceCount":bindings.iter().filter(|binding| binding["role"] == "image").count(),
        "referenceSlots":reference_slots,"referenceMode":mode,"minReferences":min,"maxReferences":refs.len(),
        "firstFrameSlot":first_slot,"lastFrameSlot":last_slot}))
}

fn prepare_graph(app: &App, file: &Path, project: &Value, step: &Step, profile: &Value, pending: &[String]) -> Result<Prepared> {
    endpoint(profile)?;
    let current = shot(project, &step.shot_id)?;
    if shot_stamp(current) != step.shot_stamp || aspect_for_take(project,&step.shot_id,&step.take,profile)? != step.aspect {
        return Err(invalid("镜头提示词已修改，请重新预检"));
    }
    if step.binding_stamp.is_some_and(|saved| saved != binding_stamp(current)) {
        return Err(invalid("镜头人物或道具绑定已修改，请重新预检"));
    }
    let mut graph = workflow_template(profile)?;
    if let Some(id) = step.take["extensionNodeId"].as_str() {
        check_extension_state(app, project, step)?;
        canvas_extensions::apply_comfy_params(app, project, id, profile, &mut graph, step.extension_state.as_ref())?;
    }
    let bindings = configured_bindings(profile, &graph)?;
    let selectors = comfy_config::selector_bindings(profile,&graph)?;
    let applied = comfy_config::apply_selectors(&mut graph,&selectors,&step.aspect)?;
    let size = comfy_config::apply_size(&mut graph,profile,&selectors,&step.aspect)?;
    let nodes = graph.as_object().unwrap();
    let has_first = nodes.values().any(|node| first_link(node).is_array());
    let has_last = nodes.values().any(|node| node["inputs"]["last_frame"].is_array() || node["inputs"]["end_frame"].is_array());
    let (width, height) = comfy_config::configured_size(profile,&step.aspect).unwrap_or_else(|| match step.aspect.as_str() {
        "9:16" => (720, 1280), "1:1" => (1024, 1024), "4:3" => (1152, 864), "3:4" => (864, 1152),
        "2:3" => (768, 1152), "3:2" => (1152, 768), "21:9" => (1344, 576), _ => (1280, 720),
    });
    let width_mapped = bindings.iter().any(|item| item["role"] == "width");
    let height_mapped = bindings.iter().any(|item| item["role"] == "height");
    if width_mapped != height_mapped || (step.take.get("aspectRatio").is_some() && !width_mapped && selectors.is_empty()) ||
        (!width_mapped && nodes.values().any(|node| {
        node["class_type"] == "ResolutionSelector" && node["inputs"]["aspect_ratio"].as_str()
            .is_some_and(|aspect| !aspect.starts_with(&step.aspect))
    })) {
        return Err(invalid("工作流画幅与镜头不匹配"));
    }
    let (first_sources,last_sources) = frame_sources(&graph);
    let connected_images: std::collections::BTreeSet<_> = nodes.values().flat_map(|node|
        node["inputs"].as_object().into_iter().flat_map(|inputs| inputs.values())
            .flat_map(|link| source_images(&graph, link))).collect();
    let mut graph = graph;
    let mut media = Vec::new();
    let mut prompt = false;
    let mut duration = false;
    let mut image_slots = Vec::new();
    let mut end_slot = None;
    let mut continuation = None;
    let mut used_targets = std::collections::HashSet::new();
    for binding in &bindings {
        let role = binding["role"].as_str();
        if matches!(role, Some("prompt" | "length" | "width" | "height" | "image" | "imageEnd")) {
            let target = (field(binding, "nodeId")?, field(binding, "field")?);
            if !used_targets.insert(target) {
                return Err(invalid("同一个工作流输入不能绑定多个角色或重复使用"));
            }
        }
        match role {
            Some("prompt") => { bind(&mut graph, binding, json!(step.prompt))?; prompt = true; }
            Some("length") if binding["durationUnit"] == "seconds" => {
                let seconds = current["duration"].as_f64().filter(|value| *value > 0.0)
                    .ok_or_else(|| invalid("镜头时长无效"))?;
                comfy_config::duration(profile,seconds)?;
                bind(&mut graph, binding, json!(seconds))?;
                duration = true;
            }
            Some("width") => bind(&mut graph, binding, json!(width))?,
            Some("height") => bind(&mut graph, binding, json!(height))?,
            Some("image") => image_slots.push(binding),
            Some("imageEnd") => end_slot = Some(binding),
            _ => {}
        }
    }
    if image_slots.iter().chain(end_slot.iter()).any(|slot| {
        let source = slot["nodeId"].as_str().map(|id| &graph[id]);
        slot["field"] != "image" || !source.is_some_and(|node| {
            node["class_type"].as_str().is_some_and(|name| name.to_ascii_lowercase().contains("loadimage"))
                && node["inputs"]["image"].is_string()
        })
    }) {
        return Err(invalid("图片映射须指向实际 LoadImage 的 image 输入；首帧须匹配 first_frame 来源"));
    }
    let first_slot = image_slots.iter().copied().find(|slot| matches_frame(slot, &first_sources));
    if has_first && (first_sources.len() != 1 || first_slot.is_none()) {
        return Err(invalid("首帧映射没有指向 first_frame 或 I2VA 首帧的唯一图片来源"));
    }
    let refs: Vec<_> = image_slots.iter().copied().filter(|slot| !matches_frame(slot,&first_sources) && !matches_frame(slot,&last_sources)).collect();
    if has_last && (last_sources.len() != 1 || !end_slot.is_some_and(|slot| matches_frame(slot, &last_sources))) {
        return Err(invalid("尾帧映射没有指向尾帧的唯一图片来源"));
    }
    if !prompt || !duration {
        return Err(invalid("请配置正向提示词与秒数时长映射"));
    }
    let mut media_inputs = Vec::new();
    let mut add = |slot: &Value, asset, role: &str| -> Result<()> {
        if slot.get("mediaMode").is_some_and(|mode| mode != "upload") {
            return Err(invalid("项目图片需要上传映射"));
        }
        bind(&mut graph, slot, json!("预检"))?;
        let path = if pending.iter().any(|id| id == asset) { PathBuf::new() }
            else { media_path(app, file, asset, "image")? };
        media.push((slot.clone(), path));
        media_inputs.push(json!({"role":role,"nodeId":slot["nodeId"],"assetId":asset}));
        Ok(())
    };
    let mode = field(&step.take,"mode")?;
    if mode == "first-last-frame" && !has_last { return Err(invalid("此工作流没有尾帧槽")); }
    let linked_refs = step.extension_state.as_ref().and_then(|state| state["referenceAssetIds"].as_array())
        .filter(|refs| !refs.is_empty());
    if linked_refs.is_some() && step.take.get("referenceAssetIds").is_some() {
        return Err(invalid("请使用节点连线或显式参考素材中的一种来源"));
    }
    let requested_refs = linked_refs.or_else(|| step.take["referenceAssetIds"].as_array())
        .or_else(|| if mode == "reference" { current["referenceAssetIds"].as_array() } else { None });
    if step.take.get("referenceAssignments").is_some() && linked_refs.is_some() {
        return Err(invalid("按槽赋值与画布参考连线只能选一种来源"));
    }
    let chosen = comfy_config::assign_references(profile,&refs,&step.take,requested_refs.map(Vec::as_slice).unwrap_or_default())?;
    if chosen.is_some() && refs.iter().any(|slot| !slot["nodeId"].as_str().is_some_and(|id| connected_images.contains(id))) {
        return Err(invalid("referenceConfig 指向未连接的图片节点"));
    }
    let references: Vec<_> = chosen.as_ref().map(|items|items.actual_ids.clone()).unwrap_or_else(||
        requested_refs.into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect());
    let cast_refs = (mode == "reference" && !refs.is_empty() && !references.iter().any(|id| pending.contains(id)))
        .then_some(references.as_slice());
    let warnings = generation_review::review_cast(project,current,&step.prompt,cast_refs,step.take.get("bindingReview"))?;
    let count = requested_refs.map_or(0, Vec::len);
    if chosen.is_none() && (count > refs.len() || (mode == "reference" && count == 0 && step.take["allowTemplateImages"] != true)) {
        return Err(invalid("参考图数量与工作流槽位不符"));
    }
    if let Some(chosen) = &chosen {
        comfy_config::verify_roles(project,chosen)?;
        for assignment in &chosen.assignments {
            let slot = refs.iter().copied().find(|slot| slot["nodeId"] == assignment["nodeId"]).unwrap();
            let id = field(assignment,"assetId")?;
            let file_result = add(slot,id,if assignment["fallback"] == true {"fallback"} else {"reference"});
            if assignment["fallback"] == true && file_result.is_err() {
                return Err(invalid(&format!("第 {} 槽的中性图 {id} 不在当前项目中或无法读取",assignment["index"])));
            }
            file_result?;
        }
    } else {
        for (slot,id) in refs.iter().zip(requested_refs.into_iter().flatten()) {
            add(slot,id.as_str().ok_or_else(|| invalid("参考图 ID 无效"))?,"reference")?;
        }
    }
    match mode {
        "reference" if !has_first && !has_last => {}
        "first-frame" | "first-last-frame" if has_first && (has_last == (mode == "first-last-frame")) => {
            let id = step.take["firstFrameAssetId"].as_str().or_else(|| current["firstFrameAssetId"].as_str())
                .ok_or_else(|| invalid("缺少首帧"))?;
            generation_review::validate_frame(app,file,project,id)?;
            add(first_slot.ok_or_else(|| invalid("缺少首帧映射"))?, id, "first-frame")?;
            if has_last {
                let id = step.take["lastFrameAssetId"].as_str().or_else(|| current["lastFrameAssetId"].as_str())
                    .ok_or_else(|| invalid("缺少尾帧"))?;
                generation_review::validate_frame(app,file,project,id)?;
                add(end_slot.ok_or_else(|| invalid("缺少尾帧映射"))?, id, "last-frame")?;
            }
        }
        "previous-tail" if has_first && !has_last => {
            if super::ffmpeg_binary().is_err() {
                return Err(invalid("请配置 DRAMLO_FFMPEG_BIN 以截取上一镜的真实尾帧"));
            }
            let previous = previous_shot(project, current)?;
            if previous["id"].as_str() != step.previous_shot_id.as_deref() { return Err(invalid("上一镜已改变")); }
            let slot = first_slot.ok_or_else(|| invalid("缺少首帧映射"))?;
            if slot.get("mediaMode").is_some_and(|mode| mode != "upload") { return Err(invalid("首帧需要上传映射")); }
            bind(&mut graph, slot, json!("预检"))?;
            continuation = Some(slot.clone());
            media_inputs.push(json!({"role":"previous-tail","nodeId":slot["nodeId"],
                "shotId":step.previous_shot_id,"assetId":step.previous_asset_id}));
        }
        "text" if refs.is_empty() && !has_first && !has_last && count == 0 => {}
        _ => return Err(invalid("工作流与所选首帧、尾帧或参考图模式不兼容")),
    }
    let template_images: Vec<_> = connected_images.iter().filter(|id| {
        !media.iter().map(|(slot,_)| slot).chain(continuation.iter()).any(|slot| slot["nodeId"] == id.as_str() && slot["field"] == "image")
    }).filter_map(|id| graph[id]["inputs"]["image"].as_str().filter(|filename| !filename.is_empty())
        .map(|filename| json!({"nodeId":id,"filename":filename}))).collect();
    if !template_images.is_empty() && step.take["allowTemplateImages"] != true {
        let labels = template_images.iter().filter_map(|item| item["filename"].as_str()).collect::<Vec<_>>().join("、");
        return Err(invalid(&format!("工作流仍引用固定图片：{labels}。补齐项目图片，或确认允许保留工作流固定图")));
    }
    let output = profile["adapter"]["outputConfig"].is_object().then(|| json!({
        "aspectRatio":step.aspect,"size":size,"appliedBindings":applied,"duration":current["duration"]
    }));
    let reference_assignments = chosen.map(|items|items.assignments).unwrap_or_default();
    let reference_facts = generation_review::reference_facts(project,current,&references);
    let size = size.or_else(|| width_mapped.then(|| json!({"width":width,"height":height,"mode":"mapped-pixels"})));
    let workload = generation_review::workload("comfyui",1,media.len()+usize::from(continuation.is_some())+template_images.len(),
        current["duration"].as_f64().unwrap_or_default(),json!(size));
    Ok(Prepared { graph, media, continuation, template_images, warnings, reference_assignments, media_inputs, reference_facts, workload, output })
}

pub(super) async fn http(profile: &Value, method: &str, route: &str, content_type: &str, body: Vec<u8>) -> Result<(StatusCode, String, Bytes)> {
    let address = endpoint(profile)?;
    let operation = async {
        let stream = tokio::net::TcpStream::connect(address).await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
            .map_err(|_| invalid("无法建立 ComfyUI 请求"))?;
        tokio::spawn(async move { let _ = connection.await; });
        let mut request = hyper::Request::builder().method(method).uri(route).header(header::HOST, address.to_string())
            .header(header::CONTENT_TYPE, content_type).header(header::CONNECTION, "close");
        if let Some(key) = profile["apiKey"].as_str().filter(|key| !key.is_empty()) {
            request = request.header(header::AUTHORIZATION, format!("Bearer {key}"));
        }
        let reply = sender.send_request(request.body(Full::new(Bytes::from(body))).map_err(|_| invalid("ComfyUI 请求无效"))?).await
            .map_err(|_| invalid("ComfyUI 响应中断"))?;
        let status = reply.status();
        let content_type = reply.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream").to_owned();
        let bytes = Limited::new(reply.into_body(), MAX_RESPONSE).collect().await
            .map_err(|_| invalid("ComfyUI 响应不完整或过大"))?.to_bytes();
        Ok::<_, Failure>((status, content_type, bytes))
    };
    tokio::time::timeout(Duration::from_secs(90), operation).await.map_err(|_| invalid("ComfyUI 请求超时"))?
}

pub(super) fn json_body(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| invalid("ComfyUI 响应不是 JSON"))
}

pub(super) async fn upload(profile: &Value, source: &Path, label: &str) -> Result<String> {
    let extension = source.extension().and_then(|ext| ext.to_str()).unwrap_or_default().to_ascii_lowercase();
    if !["png", "jpg", "jpeg", "webp"].contains(&extension.as_str()) {
        return Err(invalid("当前图片格式不支持上传"));
    }
    let boundary = format!("studio-{label}");
    let mut bytes = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{label}.{extension}\"\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    if fs::metadata(source)?.len() > 48 * 1024 * 1024 {
        return Err(invalid("图片素材过大"));
    }
    bytes.extend(fs::read(source)?);
    bytes.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
    let (status, _, body) = http(profile, "POST", "/upload/image", &format!("multipart/form-data; boundary={boundary}"), bytes).await?;
    if !status.is_success() { return Err(invalid("上传图片失败")); }
    let result = json_body(&body)?;
    let name = field(&result, "name")?;
    let dir = result["subfolder"].as_str().unwrap_or_default();
    Ok(if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") })
}

pub(super) async fn last_frame(video: &Path) -> Result<tempfile::TempDir> {
    let binary = super::ffmpeg_binary()?;
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("tail.png");
    let mut command = tokio::process::Command::new(binary);
    command.kill_on_drop(true).args(["-nostdin", "-v", "error", "-sseof", "-1", "-i"]).arg(video)
        .args(["-an", "-update", "1", "-y"]).arg(&output);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let status = tokio::time::timeout(Duration::from_secs(60), command.status()).await
        .map_err(|_| invalid("截取尾帧超时"))??;
    if !status.success() || !output.is_file() { return Err(invalid("无法截取真实尾帧")); }
    Ok(directory)
}

fn continuation_asset(batch: &Batch, step: &Step, project: &Value) -> Result<String> {
    let previous = previous_shot(project, shot(project, &step.shot_id)?)?;
    if previous["id"].as_str() != step.previous_shot_id.as_deref() {
        return Err(invalid("上一镜已改变"));
    }
    let selected = field(previous, "videoAssetId")?;
    let expected = step.previous_asset_id.as_deref().or_else(|| batch.steps.iter()
        .find(|other| Some(other.shot_id.as_str()) == step.previous_shot_id.as_deref() && other.status == "saved")
        .map(|other| other.asset_id.as_str()));
    if expected != Some(selected) { return Err(invalid("上一镜选片已改变，请重新预检")); }
    Ok(selected.to_owned())
}

pub(super) fn escape(value: &str) -> String {
    value.bytes().map(|byte| format!("%{byte:02X}")).collect()
}

fn archive(app: &App, batch: &Batch, step: &Step, bytes: &[u8], extension: &str) -> Result<VideoBinding> {
    let _lock = app.writes.lock().map_err(|_| invalid("项目写入锁不可用"))?;
    let (file, mut project) = studio_mcp::project_by_id(app, &batch.project_id)?;
    let folder = file.parent().ok_or_else(|| invalid("项目目录无效"))?;
    let relative = format!("assets/videos/{}.{extension}", step.asset_id);
    let path = scoped_path(&app.root, &folder.join(&relative))?;
    let manifest_path = scoped_path(&app.root, &folder.join("assets/manifest.json"))?;
    let mut manifest: Vec<Value> = if manifest_path.exists() {
        serde_json::from_slice(&fs::read(&manifest_path)?).map_err(|_| invalid("资产清单无效"))?
    } else { Vec::new() };
    if !manifest.iter().any(|item| item["id"] == step.asset_id) {
        atomic_save(&path, bytes)?;
        manifest.push(json!({
            "id": step.asset_id, "kind": "video", "filename": format!("{}.{extension}", step.asset_id),
            "mimeType": format!("video/{extension}"), "size": bytes.len(), "createdAt": now(),
            "source": "ai-generated", "prompt": step.prompt, "projectId": batch.project_id, "shotId": step.shot_id,
            "fileRelPath": relative, "width": 0, "height": 0, "duration": 0, "fps": 0, "hasAudio": false,
            "videoGeneration": { "batchId": batch.request_id, "profileId": step.profile_id, "promptId": step.prompt_id, "previousShotId": step.previous_shot_id, "previousAssetId": step.previous_asset_id,
                "extensionNodeId":step.take["extensionNodeId"],"extensionState":step.extension_state }
        }));
        atomic_save(&manifest_path, &serde_json::to_vec_pretty(&manifest).unwrap())?;
    } else if !path.is_file() {
        return Err(invalid("已归档的原片文件缺失"));
    }
    let extension_matches = step.take["extensionNodeId"].is_null() ||
        (check_extension_state(app, &project, step).is_ok()
            && get_profile(app, Some(&step.profile_id)).is_ok_and(|profile| profile_stamp(&profile) == step.profile_stamp));
    let mut project_changed = false;
    if let Some(id) = step.take["extensionNodeId"].as_str().filter(|_| extension_matches) {
        if let Some(node) = project["canvas"]["nodes"].as_array_mut().into_iter().flatten()
            .find(|node| node["id"] == id && node["data"]["extensionExecutor"] == "studio.comfy-video") {
            node["data"]["outputAssetId"] = json!(step.asset_id);
            node["status"] = json!("success");
            project_changed = true;
        }
    }
    let aspect_matches = get_profile(app,Some(&step.profile_id)).is_ok_and(|profile|
        aspect_for_take(&project,&step.shot_id,&step.take,&profile).is_ok_and(|aspect| aspect == step.aspect));
    let mut bind = VideoBinding { applied: false, reason: "shot_missing".into() };
    if let Some(shot) = project["shots"].as_array_mut().and_then(|shots| shots.iter_mut().find(|shot| shot["id"] == step.shot_id)) {
        if shot["videoAssetId"] != step.asset_id {
            if shot["videoAssetId"].as_str().is_none() && step.matches_shot(shot) && aspect_matches && extension_matches {
                shot["videoAssetId"] = json!(step.asset_id);
                bind = VideoBinding { applied: true, reason: "empty_slot".into() };
            } else {
                bind.reason = if shot["videoAssetId"].is_string() { "existing_asset" } else { "inputs_changed" }.into();
                let mut candidates = shot["videoHistoryIds"].as_array().cloned().unwrap_or_default();
                if !candidates.contains(&json!(step.asset_id)) {
                    candidates.insert(0, json!(step.asset_id));
                    shot["videoHistoryIds"] = json!(candidates);
                }
            }
            project_changed = true;
        } else {
            bind = VideoBinding { applied: true, reason: "already_selected".into() };
        }
    }
    if let Some(position) = project["canvas"]["nodes"].as_array().and_then(|nodes| nodes.iter().position(|node|
        node["type"] == "videoGen" && node["id"] == step.shot_id && node["data"]["captureId"].is_string())) {
        let profile_matches = get_profile(app,Some(&step.profile_id)).is_ok_and(|profile| profile_stamp(&profile) == step.profile_stamp);
        let unchanged = profile_matches && aspect_matches && capture_project(&project).is_ok_and(|snapshot|
            shot(&snapshot,&step.shot_id).is_ok_and(|entity| step.matches_shot(entity)) &&
            (step.take["mode"] != "previous-tail" || continuation_asset(batch,step,&snapshot).is_ok()));
        let node = &mut project["canvas"]["nodes"][position];
        let current = node["data"]["outputAssetId"].as_str();
        if current == Some(step.asset_id.as_str()) {
            bind = VideoBinding { applied: true, reason: "already_selected".into() };
        } else if current.is_none() && unchanged {
            node["data"]["outputAssetId"] = json!(step.asset_id);
            node["status"] = json!("success");
            bind = VideoBinding { applied: true, reason: "empty_slot".into() };
        } else {
            bind.reason = if current.is_some() { "existing_asset" } else { "inputs_changed" }.into();
            let mut history = node["data"]["videoHistoryIds"].as_array().cloned().unwrap_or_default();
            if !history.contains(&json!(step.asset_id)) {
                history.insert(0, json!(step.asset_id));
                node["data"]["videoHistoryIds"] = json!(history);
            }
        }
        project_changed = true;
    }
    if project_changed {
        project["updatedAt"] = json!(now().max(project["updatedAt"].as_u64().unwrap_or(0).saturating_add(1)));
        atomic_save(&file, &serde_json::to_vec_pretty(&project).unwrap())?;
        notify_project_revision(app,json!({"projectId":batch.project_id,"updatedAt":project["updatedAt"]}));
    }
    Ok(bind)
}

fn next_status(app: &App, batch: &mut Batch, index: usize, status: &str, prompt_id: Option<String>) -> Result<()> {
    let _lock = app.writes.lock().map_err(|_| invalid("批次写锁不可用"))?;
    write_step_status(app, batch, index, status, prompt_id)
}

fn write_step_status(app: &App, batch: &mut Batch, index: usize, status: &str, prompt_id: Option<String>) -> Result<()> {
    let persisted = read_batch(app, &batch.project_id, &batch.request_id)?;
    batch.pause_requested = persisted.pause_requested;
    batch.stop_wait_requested = persisted.stop_wait_requested;
    batch.cancelled_prompt_id = persisted.cancelled_prompt_id;
    batch.steps[index].status = status.into();
    if let Some(prompt_id) = prompt_id { batch.steps[index].prompt_id = Some(prompt_id); }
    write_batch(app, batch)
}

async fn execute_step(app: &App, batch: &mut Batch, index: usize) -> Result<()> {
    let step = batch.steps[index].clone();
    let profile = get_profile(app, Some(&step.profile_id))?;
    if profile_stamp(&profile) != step.profile_stamp { return Err(invalid("工作流配置已改变")); }
    let prompt_id = if let Some(id) = step.prompt_id.clone() { id } else {
        let (file, project) = project_file(app, &batch.project_id)?;
        let selected = get_profile(app, chosen_profile(shot(&project, &step.shot_id)?, &step.take))?;
        if selected["id"] != step.profile_id { return Err(invalid("镜头的视频接入已改变")); }
        let mut prepared = prepare_graph(app, &file, &project, &step, &profile, &[])?;
        for (i, (slot, path)) in prepared.media.iter().enumerate() {
            let name = upload(&profile, path, &format!("{}-{i}", step.asset_id)).await?;
            bind(&mut prepared.graph, slot, json!(name))?;
        }
        if let Some(slot) = prepared.continuation {
            let previous_id = continuation_asset(batch, &step, &project)?;
            let video = media_path(app, &file, &previous_id, "video")?;
            let frames = last_frame(&video).await?;
            let name = upload(&profile, &frames.path().join("tail.png"), &format!("{}-tail", step.asset_id)).await?;
            bind(&mut prepared.graph, &slot, json!(name))?;
            batch.steps[index].previous_asset_id = Some(previous_id);
        }
        {
            let _lock = app.writes.lock().map_err(|_| invalid("批次写锁不可用"))?;
            let (_, latest) = project_file(app, &batch.project_id)?;
            check_extension_state(app, &latest, &step)?;
            let latest_shot = shot(&latest, &step.shot_id)?;
            let latest_profile = get_profile(app, chosen_profile(latest_shot, &step.take))?;
            if latest_profile["id"] != step.profile_id || !step.matches_shot(latest_shot) || aspect_for_take(&latest,&step.shot_id,&step.take,&latest_profile)? != step.aspect ||
                profile_stamp(&latest_profile) != step.profile_stamp {
                return Err(invalid("上传期间镜头或工作流改变，请重新预检"));
            }
            if let Some(asset) = &batch.steps[index].previous_asset_id {
                if &continuation_asset(batch, &step, &latest)? != asset { return Err(invalid("上一镜选片已变化")); }
            }
            write_step_status(app, batch, index, "submitting", None)?;
        }
        let payload = json!({ "prompt": prepared.graph, "client_id": step.asset_id });
        let (status, _, body) = http(&profile, "POST", "/prompt", "application/json", serde_json::to_vec(&payload).unwrap()).await?;
        if status == StatusCode::BAD_REQUEST {
            next_status(app, batch, index, "failed", None)?;
            return Err(invalid("ComfyUI 拒绝工作流"));
        }
        if !status.is_success() { return Err(invalid("ComfyUI 提交结果未知")); }
        let id = field(&json_body(&body)?, "prompt_id")?.to_owned();
        next_status(app, batch, index, "waiting", Some(id.clone()))?;
        id
    };
    for _ in 0..900 {
        let state = read_batch(app, &batch.project_id, &batch.request_id)?;
        if state.cancelled_prompt_id.as_deref() == Some(&prompt_id) {
            next_status(app, batch, index, "failed", None)?;
            return Err(invalid("已请求停止当前镜头"));
        }
        if state.stop_wait_requested { return Err(invalid("已停止本地等待，可按原任务 ID 继续")); }
        let (status, _, body) = http(&profile, "GET", &format!("/history/{}", escape(&prompt_id)), "application/json", Vec::new()).await?;
        if status.is_success() {
            let history = json_body(&body)?;
            let entry = &history[&prompt_id];
            if entry["status"]["status_str"] == "error" {
                next_status(app, batch, index, "failed", None)?;
                return Err(invalid("ComfyUI 生成失败"));
            }
            if let Some(outputs) = entry["outputs"].as_object() {
                for result in outputs.values().filter_map(Value::as_object).flat_map(|value| value.values().filter_map(Value::as_array).flatten()) {
                    let Some(name) = result["filename"].as_str() else { continue };
                    let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
                    if !["mp4", "webm"].contains(&ext.as_str()) { continue; }
                    let route = format!("/view?filename={}&subfolder={}&type={}", escape(name),
                        escape(result["subfolder"].as_str().unwrap_or_default()), escape(result["type"].as_str().unwrap_or("output")));
                    let (status, _, bytes) = http(&profile, "GET", &route, "application/octet-stream", Vec::new()).await?;
                    let video_signature = if ext == "mp4" { bytes.get(4..8) == Some(b"ftyp") }
                        else { bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) };
                    if !status.is_success() || !video_signature { return Err(invalid("ComfyUI 原片读取失败或文件格式不符")); }
                    let bind = archive(app, batch, &batch.steps[index], &bytes, &ext)?;
                    batch.steps[index].bind = Some(bind);
                    return next_status(app, batch, index, "saved", None);
                }
                if entry["status"]["completed"] == true {
                    return Err(invalid("ComfyUI 未返回 MP4 或 WebM"));
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(invalid("等待 ComfyUI 超时，可继续查询原任务"))
}

async fn worker(app: App, mut batch: Batch) {
    for index in 0..batch.steps.len() {
        if batch.steps[index].status == "saved" { continue; }
        if read_batch(&app, &batch.project_id, &batch.request_id).map(|job| job.pause_requested).unwrap_or(true) {
            batch.status = "paused".into();
            break;
        }
        if let Err(error) = execute_step(&app, &mut batch, index).await {
            batch.error = Some(error.1);
            batch.status = match batch.steps[index].status.as_str() {
                "submitting" => "unknown",
                "waiting" => "paused",
                _ => "failed"
            }.into();
            break;
        }
    }
    if batch.steps.iter().all(|step| step.status == "saved") { batch.status = "succeeded".into(); }
    if let Ok(mut active) = app.video_active.lock() {
        if let Ok(_lock) = app.writes.lock() { let _ = write_batch(&app, &batch); }
        *active = None;
    }
}

fn check_extension_state(app: &App, project: &Value, step: &Step) -> Result<()> {
    if let Some(id) = step.take["extensionNodeId"].as_str() {
        if canvas_extensions::comfy_state(app, project, id, &step.profile_id)? != step.extension_state {
            return Err(Failure::new(StatusCode::CONFLICT, "扩展节点参数或连线输出已变化，请重新预检".into()));
        }
    }
    Ok(())
}

fn build_step(app: &App, file: &Path, project: &Value, takes: &[Value], take: &Value, index: usize, id: &str, pending: &[String]) -> Result<Step> {
    let shot_id = field(take, "shotId")?;
    let entity = shot(project, shot_id)?;
    if entity["canvasSubmissionPending"] == true {
        return Err(invalid("此连续拍节点有旧版未确认任务，请先核对原任务结果"));
    }
    let final_prompt = entity["compiledVideoPrompt"].as_str().map(str::trim).filter(|text| !text.is_empty())
        .ok_or_else(|| invalid("缺少最终视频提示词，请用 studio_set_video_prompt 保存"))?;
    let profile_id = chosen_profile(entity, take);
    let profile = get_profile(app, profile_id)?;
    let extension_state = match take["extensionNodeId"].as_str() {
        Some(id) => canvas_extensions::comfy_state(app, project, id, field(&profile, "id")?)?,
        None => None,
    };
    let previous = if take["mode"] == "previous-tail" { Some(previous_shot(project, entity)?) } else { None };
    let previous_shot_id = previous.and_then(|item| item["id"].as_str()).map(str::to_owned);
    let previous_asset_id = previous.and_then(|item| item["videoAssetId"].as_str()).map(str::to_owned);
    if let Some(asset) = previous_asset_id.as_deref() { media_path(app, file, asset, "video")?; }
    if previous_shot_id.is_some() && previous_asset_id.is_none() &&
        !takes.iter().any(|item| item["shotId"].as_str() == previous_shot_id.as_deref()) {
        return Err(invalid("上一镜还没有选片，也不在本批次"));
    }
    let mut step = Step {
        shot_id: shot_id.into(), title: entity["title"].clone(), prompt: final_prompt.into(),
        profile_id: field(&profile, "id")?.into(), profile_stamp: profile_stamp(&profile),
        shot_stamp: shot_stamp(entity), binding_stamp: Some(binding_stamp(entity)),
        aspect: aspect_for_take(project,shot_id,take,&profile)?.into(), previous_shot_id, previous_asset_id, status: "ready".into(),
        prompt_id: None, asset_id: format!("video_{id}_{index}"), take: take.clone(), template_images: vec![], warnings: vec![], reference_assignments: vec![], media_inputs: vec![], reference_facts: Value::Null, workload: Value::Null, output: None, bind: None, extension_state,
    };
    let prepared = prepare_graph(app, file, project, &step, &profile, pending)?;
    step.template_images = prepared.template_images;
    step.warnings = prepared.warnings;
    step.reference_assignments = prepared.reference_assignments;
    step.media_inputs = prepared.media_inputs;
    step.reference_facts = prepared.reference_facts;
    step.workload = prepared.workload;
    step.output = prepared.output;
    Ok(step)
}

pub(super) fn inspect_plan(app: &App, file: &Path, project: &Value, takes: &[Value], take: &Value, pending: &[String]) -> Result<Value> {
    let projected = capture_project(project)?;
    let project = &*projected;
    let step = build_step(app,file,project,takes,take,0,"inspect",pending)?;
    let profile = get_profile(app,Some(&step.profile_id))?;
    let entity = shot(project,&step.shot_id)?;
    Ok(json!({"profileId":step.profile_id,"workflow":describe_workflow(&profile)?,
        "approvalStamp":stamp(&json!([step.profile_stamp,step.prompt,step.aspect,entity["duration"],
            entity["sceneId"],entity["chapterId"],entity["index"],step.previous_shot_id])),
        "previousShotId":step.previous_shot_id,"previousAssetId":step.previous_asset_id,"warnings":step.warnings,
        "templateImages":step.template_images,"referenceAssignments":step.reference_assignments,"mediaInputs":step.media_inputs,
        "referenceFacts":step.reference_facts,"workload":step.workload,"output":step.output}))
}

pub(super) fn shot_change_context(app: &App, project_id: &str, active: Option<&(String, String)>) -> Result<Value> {
    let pending = kv_path(app, &format!("video-latest:{}", json!(project_id)))?;
    let mut blocked = active.is_some_and(|(project, _)| project == project_id);
    let mut current = None;
    match fs::read_to_string(pending) {
        Ok(id) => {
            let mut batch = read_batch(app, project_id, &id)?;
            normalize_interrupted(&mut batch, blocked);
            blocked |= matches!(batch.status.as_str(), "running" | "paused" | "unknown");
            if blocked { current = Some(batch.request_id); }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let prepared = match fs::read_to_string(kv_path(app, &format!("video-prepared:{}", json!(project_id)))?) {
        Ok(id) if read_batch(app, project_id, &id)?.status == "prepared" => Some(id),
        Ok(_) => None,
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    Ok(json!({"blocked":blocked,"activeRequestId":current,"preparedRequestId":prepared}))
}

pub(super) fn invoke(app: &App, name: &str, input: &Value) -> Result<Value> {
    let mut active = app.video_active.lock().map_err(|_| invalid("视频队列锁不可用"))?;
    let _lock = app.writes.lock().map_err(|_| invalid("批次写锁不可用"))?;
    let project_id = field(input, "projectId")?;
    if name == "studio_check_video_readiness" {
        let (file, project) = project_file(app, project_id)?;
        if input.get("expectedUpdatedAt").is_some() {
            let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
            validate_project_revision(input,revision,true)?;
        }
        let takes = input["shots"].as_array().filter(|items| !items.is_empty() && items.len() <= 30)
            .ok_or_else(|| invalid("请提供 1 到 30 个分镜及生成方式"))?;
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        for (index, take) in takes.iter().enumerate() {
            let shot_id = field(take, "shotId")?;
            if !seen.insert(shot_id) { return Err(invalid("镜头重复")); }
            let step = build_step(app, &file, &project, takes, take, index, "readiness", &[]);
            let entity = shot(&project, shot_id).ok();
            let warnings = entity.map(|shot| project_authoring::missing_prop_images(&project, shot)).unwrap_or_default();
            let sources = step.as_ref().ok().map(|step| json!({
                "previousShotId":step.previous_shot_id,"previousVideoAssetId":step.previous_asset_id,
                "firstFrameAssetId":take["firstFrameAssetId"].as_str().or_else(|| entity.and_then(|shot| shot["firstFrameAssetId"].as_str())).filter(|_| matches!(take["mode"].as_str(),Some("first-frame"|"first-last-frame"))),
                "lastFrameAssetId":take["lastFrameAssetId"].as_str().or_else(|| entity.and_then(|shot| shot["lastFrameAssetId"].as_str())).filter(|_| take["mode"] == "first-last-frame")
            }));
            items.push(json!({"shotId":shot_id,"profileId":step.as_ref().ok().map(|step| &step.profile_id),
                "ready":step.is_ok(),"issue":step.as_ref().err().map(|error|&error.1),
                "inputMode":take["mode"],"referenceAssignments":step.as_ref().ok().map(|step| &step.reference_assignments),
                "mediaInputs":step.as_ref().ok().map(|step| &step.media_inputs),
                "referenceFacts":step.as_ref().ok().map(|step| &step.reference_facts),
                "workload":step.as_ref().ok().map(|step| &step.workload),
                "output":step.as_ref().ok().and_then(|step|step.output.as_ref()),"templateImages":step.as_ref().ok().map(|step| &step.template_images),
                "issues":step.as_ref().err().map(|error| &error.3),"warnings":step.as_ref().ok().map(|step| &step.warnings),"frameSources":sources,"missingPropImages":warnings}));
        }
        return Ok(json!({"projectId":project_id,"updatedAt":project["updatedAt"],"ready":items.iter().all(|item| item["ready"] == true),"items":items}));
    }
    let pending = kv_path(app, &format!("video-latest:{}", json!(project_id)))?;
    let prepared_path = kv_path(app, &format!("video-prepared:{}", json!(project_id)))?;
    let is_active = |id: &str| active.as_ref().is_some_and(|(project, request)| project == project_id && request == id);
    if name == "studio_current_video_batch" {
        let pending_id = match fs::read_to_string(&pending) {
            Ok(id) => Some(id),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut pending_batch = if let Some(id) = pending_id.as_deref() {
            let mut job = read_batch(app, project_id, id)?;
            normalize_interrupted(&mut job, is_active(id));
            Some(job)
        } else { None };
        if !pending_batch.as_ref().is_some_and(|job| matches!(job.status.as_str(), "running" | "unknown" | "paused")) {
            if let Ok(id) = fs::read_to_string(prepared_path) {
                let prepared = read_batch(app, project_id, &id)?;
                if prepared.status == "prepared" { pending_batch = Some(prepared); }
            }
        }
        return Ok(json!({ "batch": pending_batch.as_ref().map(|batch| response(app,batch)) }));
    }
    let id = field(input, "requestId")?;
    if id.len() != 36 || !id.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-') {
        return Err(invalid("请求 ID 必须使用 UUID"));
    }
    let mut batch = if name == "studio_prepare_video" && !job_path(app, project_id, id)?.exists() {
        let (file, project) = project_file(app, project_id)?;
        let old_revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
        validate_project_revision(input, old_revision, true)?;
        let takes = input["shots"].as_array().filter(|takes| !takes.is_empty() && takes.len() <= 30)
            .ok_or_else(|| invalid("请提供 1 到 30 个分镜"))?;
        let mut steps = Vec::new();
        for (index, take) in takes.iter().enumerate() {
            let shot_id = field(take, "shotId")?;
            if steps.iter().any(|step: &Step| step.shot_id == shot_id) { return Err(invalid("镜头重复")); }
            let step = build_step(app, &file, &project, takes, take, index, id, &[])?;
            steps.push(step);
        }
        steps.sort_by_key(|step| shot(&project, &step.shot_id).ok().and_then(|shot| shot["index"].as_u64()).unwrap_or(u64::MAX));
        let batch = Batch { project_id: project_id.into(), request_id: id.into(), revision: old_revision,
            status: "prepared".into(), pause_requested: false, stop_wait_requested: false, cancelled_prompt_id: None, steps, error: None };
        write_batch(app, &batch)?;
        atomic_save(&prepared_path, id.as_bytes())?;
        return Ok(json!({ "batch": response(app,&batch), "reused": false,
            "playApproved": project["playApproved"] == true }));
    } else {
        read_batch(app, project_id, id)?
    };
    normalize_interrupted(&mut batch, is_active(id));
    // 剧本未确认时把状态带回去：确认是软约束，由调用方决定要不要先问用户。
    let approved = project_file(app, project_id)
        .map(|(_, project)| project["playApproved"] == true)
        .unwrap_or(false);
    match name {
        "studio_video_status" | "studio_prepare_video" => Ok(json!({ "batch": response(app,&batch), "reused": true,
            "playApproved": approved })),
        "studio_discard_video_plan" => {
            let reused = batch.status == "discarded";
            if !reused {
                if batch.status != "prepared" || batch.steps.iter().any(|step| step.prompt_id.is_some()) {
                    return Err(invalid("只能撤销尚未提交的预检计划"));
                }
                batch.status = "discarded".into();
                for step in &mut batch.steps { step.status = "discarded".into(); }
                write_batch(app, &batch)?;
            }
            Ok(json!({"batch":response(app,&batch),"reused":reused}))
        }
        "studio_pause_video" | "studio_stop_video_wait" => {
            if batch.status != "running" || !is_active(id) { return Err(invalid("批次未运行")); }
            batch.pause_requested = true;
            batch.stop_wait_requested = name == "studio_stop_video_wait";
            write_batch(app, &batch)?;
            Ok(json!({ "batch": response(app,&batch) }))
        }
        "studio_confirm_video" | "studio_resume_video" => {
            if input["confirmed"] != true { return Err(invalid("请先确认计划")); }
            if batch.status == "discarded" { return Err(invalid("此计划已撤销，请用新请求 ID 重新预检")); }
            let expected = if name == "studio_resume_video" { "paused" } else { "prepared" };
            if batch.status != expected { return Ok(json!({ "batch": response(app,&batch), "reused": true })); }
            if active.is_some() { return Err(Failure::new(StatusCode::LOCKED, "本机队列已有制作任务运行".into())); }
            if let Ok(previous) = fs::read_to_string(&pending) {
                if previous != id {
                    let mut previous = read_batch(app, project_id, &previous)?;
                    normalize_interrupted(&mut previous, false);
                    if matches!(previous.status.as_str(), "unknown" | "paused") {
                        return Err(invalid("请先核对该项目此前的制作任务"));
                    }
                }
            }
            let (_, project) = project_file(app, project_id)?;
            if expected == "prepared" && project["updatedAt"].as_u64() != Some(batch.revision) {
                let mut conflict = Failure::revision_conflict(project["updatedAt"].as_u64(), Some(batch.revision));
                conflict.1 = "项目已更新，请重新预检".into();
                return Err(conflict);
            }
            for step in &batch.steps {
                if step.prompt_id.is_none() && step.status != "saved" {
                    get_profile(app, Some(&step.profile_id))?;
                    check_extension_state(app, &project, step)?;
                }
            }
            batch.status = "running".into();
            batch.pause_requested = false;
            batch.stop_wait_requested = false;
            batch.error = None;
            write_batch(app, &batch)?;
            atomic_save(&pending, id.as_bytes())?;
            *active = Some((project_id.into(), id.into()));
            tokio::spawn(worker(app.clone(), batch.clone()));
            Ok(json!({ "batch": response(app,&batch), "reused": false }))
        }
        _ => Err(invalid("未知视频队列操作")),
    }
}

fn normalize_interrupted(batch: &mut Batch, active: bool) {
    if batch.status == "running" && !active {
        batch.status = if batch.steps.iter().any(|step| step.status == "submitting") { "unknown" } else { "paused" }.into();
    }
}


#[derive(Deserialize)]
pub(super) struct BrowserCall {
    name: String,
    input: Value,
}

pub(super) async fn browser(State(app): State<App>, Json(call): Json<BrowserCall>) -> Result<Json<Value>> {
    if !METHODS.contains(&call.name.as_str()) {
        return Err(invalid("未知制作操作"));
    }
    if call.name == "studio_cancel_video_shot" {
        field(&call.input, "expectedPromptId")?;
    }
    Ok(Json(dispatch(&app, &call.name, &call.input).await?))
}

pub(super) async fn dispatch(app: &App, name: &str, input: &Value) -> Result<Value> {
    if name == "studio_cancel_video_shot" {
        return cancel_current(app, input).await;
    }
    if name == "studio_recover_video_id" {
        return recover_remote(app, input).await;
    }
    invoke(app, name, input)
}

async fn cancel_current(app: &App, input: &Value) -> Result<Value> {
    if input["confirmed"] != true { return Err(invalid("请先确认停止当前镜头")); }
    let project = field(input, "projectId")?;
    let request = field(input, "requestId")?;
    let expected_prompt = input.get("expectedPromptId")
        .map(|_| field(input, "expectedPromptId")).transpose()?;
    let (profile, prompt_id) = {
        let active = app.video_active.lock().map_err(|_| invalid("批次锁不可用"))?;
        let _lock = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
        if !active.as_ref().is_some_and(|(p, r)| p == project && r == request) {
            return Err(invalid("批次当前未运行"));
        }
        let mut batch = read_batch(app, project, request)?;
        let step = batch.steps.iter().find(|step| step.status == "waiting");
        if expected_prompt.is_some() && step.and_then(|item| item.prompt_id.as_deref()) != expected_prompt {
            return Err(Failure::new(StatusCode::CONFLICT, "当前镜头已变化，请重新核对后确认停止".into()));
        }
        let step = step.ok_or_else(|| invalid("缺少远端任务 ID"))?;
        let id = step.prompt_id.clone().ok_or_else(|| invalid("缺少远端任务 ID"))?;
        let profile = get_profile(app, Some(&step.profile_id))?;
        if profile_stamp(&profile) != step.profile_stamp { return Err(invalid("视频接入已改变")); }
        batch.pause_requested = true;
        write_batch(app, &batch)?;
        (profile, id)
    };
    let (status, _, body) = http(&profile, "POST", &format!("/api/jobs/{}/cancel", escape(&prompt_id)), "application/json", b"{}".to_vec()).await?;
    if !status.is_success() { return Err(invalid("ComfyUI 未接受定向取消，后续镜头暂停")); }
    let cancelled = json_body(&body)?["cancelled"] == true;
    let _lock = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
    let mut batch = read_batch(app, project, request)?;
    if cancelled && batch.steps.iter().any(|step| step.status == "waiting" && step.prompt_id.as_deref() == Some(&prompt_id)) {
        batch.cancelled_prompt_id = Some(prompt_id);
        write_batch(app, &batch)?;
    }
    Ok(json!({ "batch": response(app,&batch), "cancelled": cancelled }))
}

async fn recover_remote(app: &App, input: &Value) -> Result<Value> {
    let project = field(input, "projectId")?;
    let request = field(input, "requestId")?;
    let prompt_id = field(input, "promptId")?;
    let (profile, tag) = {
        let active = app.video_active.lock().map_err(|_| invalid("批次锁不可用"))?;
        let mut batch = read_batch(app, project, request)?;
        if active.as_ref().is_some_and(|(p, r)| p == project && r == request) {
            return Err(invalid("批次正在运行"));
        }
        normalize_interrupted(&mut batch, false);
        let step = batch.steps.iter().find(|step| step.status == "submitting")
            .filter(|_| batch.status == "unknown").ok_or_else(|| invalid("没有未知提交"))?;
        let profile = get_profile(app, Some(&step.profile_id))?;
        if profile_stamp(&profile) != step.profile_stamp { return Err(invalid("视频接入已改变")); }
        (profile, step.asset_id.clone())
    };
    let (status, _, bytes) = http(&profile, "GET", &format!("/history/{}", escape(prompt_id)), "application/json", Vec::new()).await?;
    let mut matches = status.is_success() && json_body(&bytes)?[prompt_id]["prompt"][3]["client_id"] == tag;
    if !matches {
        let (status, _, bytes) = http(&profile, "GET", "/queue", "application/json", Vec::new()).await?;
        if status.is_success() {
            let queue = json_body(&bytes)?;
            matches = ["queue_running", "queue_pending"].into_iter()
                .filter_map(|key| queue[key].as_array()).flatten()
                .any(|item| item[1] == prompt_id && item[3]["client_id"] == tag);
        }
    }
    if !matches { return Err(invalid("远端任务不属于这个镜头，保留未知状态")); }
    let _lock = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
    let mut batch = read_batch(app, project, request)?;
    let step = batch.steps.iter_mut().find(|step| step.asset_id == tag && step.status == "submitting")
        .ok_or_else(|| invalid("批次状态已经改变"))?;
    step.prompt_id = Some(prompt_id.into());
    step.status = "waiting".into();
    batch.status = "paused".into();
    batch.error = None;
    write_batch(app, &batch)?;
    Ok(json!({ "batch": response(app,&batch) }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ComfyCall {
    url: String,
    method: Option<String>,
    body: Option<String>,
    body_base64: Option<String>,
    content_type: Option<String>,
}

pub(super) async fn browser_comfy(State(app): State<App>, Json(call): Json<ComfyCall>) -> Result<Json<Value>> {
    let uri: axum::http::Uri = call.url.parse().map_err(|_| invalid("ComfyUI 地址无效"))?;
    let authority = uri.authority().ok_or_else(|| invalid("ComfyUI 地址无效"))?;
    let base = format!("http://{authority}");
    if uri.scheme_str() != Some("http") || !["127.0.0.1", "localhost"].contains(&authority.host()) {
        return Err(invalid("Web 手动生成使用本机原生 ComfyUI"));
    }
    let method = call.method.as_deref().unwrap_or("GET");
    let path = uri.path();
    let allowed = match method {
        "GET" => matches!(path, "/view" | "/system_stats" | "/object_info" | "/userdata" | "/queue")
            || ["/history/", "/object_info/", "/userdata/"].iter().any(|prefix| path.starts_with(prefix)),
        "POST" => matches!(path, "/prompt" | "/upload/image" | "/upload/audio" | "/upload/mask"),
        _ => false,
    };
    if !allowed { return Err(invalid("此路径不属于 Web 工作台的原生 ComfyUI 接口")); }
    if method == "POST" && path == "/prompt" && app.video_active.lock().map_err(|_| invalid("任务锁不可用"))?.is_some() {
        return Err(invalid("本机已在执行制作批次，请先等待或暂停"));
    }
    let record: Value = serde_json::from_slice(&fs::read(kv_path(&app, "settings")?)?)
        .map_err(|_| invalid("工作台模型设置无效"))?;
    let settings: Value = serde_json::from_str(record["value"].as_str().unwrap_or_default())
        .map_err(|_| invalid("工作台模型设置无效"))?;
    let profiles = ["images", "videos"].into_iter().filter_map(|key| settings[key].as_array()).flatten();
    let profile = profiles.filter(|profile| profile["adapter"]["type"] == "comfyui-prompt")
        .find(|profile| profile["baseUrl"].as_str().is_some_and(|value| value.trim_end_matches('/') == base))
        .ok_or_else(|| invalid("请先在 Dramlo 保存此 ComfyUI 接入"))?;
    let body = if let Some(encoded) = call.body_base64 {
        STANDARD.decode(encoded).map_err(|_| invalid("上传内容无效"))?
    } else { call.body.unwrap_or_default().into_bytes() };
    let route = uri.path_and_query().ok_or_else(|| invalid("ComfyUI 请求缺少路径"))?;
    let (status, mime, bytes) = http(profile, method, route.as_str(),
        call.content_type.as_deref().unwrap_or("application/json"), body).await?;
    let textual = mime.starts_with("text/") || mime.contains("json") || mime.contains("xml");
    Ok(Json(json!({
        "status": status.as_u16(), "ok": status.is_success(), "contentType": mime,
        "text": if textual { String::from_utf8_lossy(&bytes).into_owned() } else { String::new() },
        "dataBase64": (!textual).then(|| STANDARD.encode(&bytes))
    })))
}
