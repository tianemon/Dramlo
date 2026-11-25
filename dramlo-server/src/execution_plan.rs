use super::*;
use std::collections::{HashMap, HashSet};

pub(super) const METHODS: &[&str] = &[
    "studio_canvas_graph", "studio_save_plan", "studio_current_plan",
    "studio_run_plan", "studio_pause_plan", "studio_capture_prepare",
];

#[derive(Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    step_id: String,
    slot: String,
}

#[derive(Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Step {
    id: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
    input: Value,
    #[serde(default)]
    sources: Vec<Source>,
    #[serde(default)]
    after: Vec<String>,
}

#[derive(Clone, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    state: String,
    #[serde(default)]
    started: bool,
    #[serde(default)]
    attempt: u32,
    #[serde(default)]
    generation: Option<u64>,
    asset_id: Option<String>,
    reason: Option<Value>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Plan {
    id: String,
    name: String,
    version: u64,
    state: String,
    pause_requested: bool,
    #[serde(default)]
    retry_failed: bool,
    #[serde(default)]
    capture: bool,
    steps: Vec<Step>,
    progress: HashMap<String, Progress>,
    #[serde(default)]
    approval: HashMap<String, Value>,
}

fn identifier<'a>(input: &'a Value, field: &str) -> Result<&'a str> {
    input[field].as_str().filter(|id| !id.is_empty() && id.len() <= 160 && !id.chars().any(char::is_control))
        .ok_or_else(|| invalid(&format!("缺少或无效的 {field}")))
}

fn data_path(app: &App, file: &Path) -> Result<PathBuf> {
    scoped_path(&app.root, &file.parent().ok_or_else(|| invalid("项目目录无效"))?.join("execution-plans.json"))
}

fn load(path: &Path) -> Result<Vec<Plan>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("执行计划记录无效")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(error.into()),
    }
}

fn save(path: &Path, plans: &[Plan]) -> Result<()> {
    atomic_save(path, &serde_json::to_vec_pretty(plans).map_err(|_| invalid("无法保存执行计划"))?)
}

fn request_id(project: &str, plan: &Plan, step: &Step) -> String {
    let progress = plan.progress.get(&step.id);
    let attempt = progress.map_or(0,|item|item.attempt);
    let generation = progress.and_then(|item|item.generation).unwrap_or(plan.version);
    let key = json!([project,plan.id,generation,step.id,attempt]).to_string();
    let hash = |bytes: &mut dyn Iterator<Item=u8>| bytes.fold(0xcbf29ce484222325_u64,|hash,byte|
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3));
    let raw = format!("{:016x}{:016x}", hash(&mut key.bytes()), hash(&mut key.bytes().rev()));
    format!("{}-{}-{}-{}-{}", &raw[..8], &raw[8..12], &raw[12..16], &raw[16..20], &raw[20..])
}

fn dependencies(step: &Step) -> impl Iterator<Item = &str> {
    step.after.iter().map(String::as_str).chain(step.sources.iter().map(|source| source.step_id.as_str()))
}

fn source_index(slot: &str) -> Option<usize> {
    slot.strip_prefix("reference:")?.parse::<usize>().ok().filter(|index| *index < 10)
}

fn validate_steps(steps: Vec<Step>) -> Result<Vec<Step>> {
    if steps.is_empty() || steps.len() > 500 { return Err(invalid("计划需要 1 至 500 个步骤")); }
    let mut ids = HashMap::new();
    for step in &steps {
        if step.id.trim().is_empty() || step.id.len() > 160 || step.id.chars().any(char::is_control) ||
            ids.insert(step.id.as_str(),step.kind.as_str()).is_some() {
            return Err(invalid("步骤 ID 缺失或重复"));
        }
        let allowed: &[&str] = match step.kind.as_str() {
            "image" => &["targetType","targetId","profileId","referenceAssetIds","outputNodeId","visualPrompt","promptLayout","bindingReview","referenceAssignments","aspectRatio"],
            "video" => &["shotId","mode","profileId","referenceAssetIds","firstFrameAssetId","lastFrameAssetId","allowTemplateImages","extensionNodeId","bindingReview","referenceAssignments","aspectRatio"],
            "tail" => &[],
            "export" => &["buildFromShots","clips"],
            _ => return Err(invalid("计划步骤仅支持 image、video、tail、export")),
        };
        let input = step.input.as_object().ok_or_else(|| invalid("执行输入须为对象"))?;
        if input.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(invalid("执行输入包含未支持的字段"));
        }
        if step.kind == "image" {
            identifier(&step.input,"targetType")?;
            identifier(&step.input,"targetId")?;
        }
        if step.kind == "video" {
            identifier(&step.input,"shotId")?;
            identifier(&step.input,"mode")?;
        }
        if step.kind == "tail" && step.node_id.as_deref().is_none_or(str::is_empty) {
            return Err(invalid("尾帧步骤须指定提取视频尾帧节点 ID"));
        }
        if step.kind == "export" && step.input.get("clips").is_some() &&
            (step.input["buildFromShots"] == true || step.input["clips"].as_array().is_none_or(|clips| clips.len() > 500)) {
            return Err(invalid("导出片段列表无效，手工片段和自动组装仅能选一项"));
        }
        let mut slots = HashSet::new();
        for source in &step.sources {
            let reference = source_index(&source.slot);
            let valid = reference.is_some() && matches!(step.kind.as_str(),"image"|"video") ||
                step.kind == "video" && matches!(source.slot.as_str(),"firstFrame"|"lastFrame") ||
                step.kind == "tail" && source.slot == "video";
            let unique_slot = reference.map_or_else(|| source.slot.clone(), |index| format!("reference:{index}"));
            if !valid || !slots.insert(unique_slot) { return Err(invalid("计划输入槽位无效或重复")); }
            if source.slot == "firstFrame" && !matches!(step.input["mode"].as_str(),Some("first-frame"|"first-last-frame")) ||
                source.slot == "lastFrame" && step.input["mode"] != "first-last-frame" {
                return Err(invalid("首尾帧依赖与视频模式不匹配"));
            }
        }
    }
    for step in &steps {
        if dependencies(step).any(|id| !ids.contains_key(id)) ||
            step.sources.iter().any(|source| match ids.get(source.step_id.as_str()).copied() {
                Some("video") => source.slot != "video",
                Some("image"|"tail") => source.slot == "video",
                _ => true,
            }) {
            return Err(invalid("找不到上游步骤，或上游输出类型与图片槽不匹配"));
        }
    }
    let mut ordered = Vec::with_capacity(steps.len());
    let mut seen = HashSet::new();
    while ordered.len() < steps.len() {
        let Some(step) = steps.iter().find(|step| !seen.contains(step.id.as_str()) && dependencies(step).all(|id| seen.contains(id))) else {
            return Err(invalid("计划存在循环依赖"));
        };
        seen.insert(step.id.as_str());
        ordered.push(step.clone());
    }
    Ok(ordered)
}

fn arguments(project_id: &str, project: &Value, plan: &Plan, step: &Step, inspection: bool)
    -> Result<(Value, Vec<String>, Vec<Value>)>
{
    let mut input = step.input.clone();
    input["projectId"] = json!(project_id);
    input["expectedUpdatedAt"] = project["updatedAt"].clone();
    input["requestId"] = json!(request_id(project_id,plan,step));
    let mut pending = Vec::new();
    let mut requirements = Vec::new();
    let mut waiting = Vec::new();
    for id in &step.after {
        if plan.progress.get(id).is_none_or(|item| item.state != "succeeded") {
            waiting.push(id);
        }
    }
    let node_data = plan.capture.then(|| step.node_id.as_deref()).flatten()
        .and_then(|id| project["canvas"]["nodes"].as_array()?.iter().find(|node| node["id"] == id))
        .map(|node| &node["data"]);
    let manual_refs = node_data.and_then(|data| data["referenceAssetIds"].as_array());
    let manual_first = if input["mode"] == "first-frame" {
        node_data.and_then(|data| data["firstFrameAssetId"].as_str()).filter(|id| !id.is_empty())
    } else { None };
    let wired = |source: &Source| !(manual_refs.is_some() && source_index(&source.slot).is_some() ||
        manual_first.is_some() && source.slot == "firstFrame");
    if let Some(refs) = manual_refs { input["referenceAssetIds"] = json!(refs); }
    if let Some(asset) = manual_first { input["firstFrameAssetId"] = json!(asset); }
    for source in step.sources.iter().filter(|source| wired(source)) {
        if matches!(source.slot.as_str(),"firstFrame"|"lastFrame") {
            let upstream = plan.steps.iter().find(|item| item.id == source.step_id);
            if upstream.is_some_and(|item| {
                if item.kind != "image" { return false; }
                if item.input["targetType"] == "shot-board" { return true; }
                if item.input["targetType"] != "shot-image" { return false; }
                let stored = crate::project_authoring::resolve_shot_id(project,
                    item.input["targetId"].as_str().unwrap_or_default()).ok().and_then(|id|
                    project["shots"].as_array().into_iter().flatten().find(|shot| shot["id"] == id.as_str()));
                item.input.get("promptLayout").or_else(|| stored.and_then(|shot| shot.get("imagePromptLayout")))
                    .is_some_and(generation_review::is_grid)
            }) {
                return Err(invalid("计划把分格看板接到了首尾帧，请改接单幅图片步骤")
                    .with_issues(vec![json!({"code":"board_in_frame_slot","stepId":source.step_id,"slot":source.slot})]));
            }
        }
        let result = plan.progress.get(&source.step_id);
        let asset = result.and_then(|item| item.asset_id.clone());
        if asset.is_none() && !inspection { return Err(invalid("依赖步骤尚无已归档产物")); }
        let id = asset.clone().unwrap_or_else(|| format!("pending:{}", source.step_id));
        if asset.is_none() { pending.push(id.clone()); }
        requirements.push(json!({"slot":source.slot,"required":true,"source":source.step_id,"assetId":asset,
            "satisfied":asset.is_some(),"state":if asset.is_some() {"bound"} else {"waiting"}}));
        if let Some(index) = source_index(&source.slot) {
            let mut refs = input["referenceAssetIds"].as_array().cloned().unwrap_or_default();
            if refs.len() <= index { refs.resize(index + 1, Value::Null); }
            refs[index] = json!(id);
            input["referenceAssetIds"] = json!(refs);
        } else if source.slot == "video" {
            input["videoAssetId"] = json!(id);
        } else {
            let field = if source.slot == "firstFrame" { "firstFrameAssetId" } else { "lastFrameAssetId" };
            input[field] = json!(id);
        }
    }
    let shot = project["shots"].as_array().into_iter().flatten().find(|shot| shot["id"] == input["shotId"]);
    if let Some(refs) = input["referenceAssetIds"].as_array().or_else(|| {
        if input["mode"] == "reference" { shot.and_then(|shot| shot["referenceAssetIds"].as_array()) } else { None }
    }) {
        for (index, asset) in refs.iter().enumerate() {
            if !step.sources.iter().filter(|source| wired(source)).any(|source| source_index(&source.slot) == Some(index)) {
                requirements.push(json!({"slot":format!("reference:{index}"),"required":true,"assetId":asset,
                    "satisfied":asset.is_string(),"state":if asset.is_string() {"bound"} else {"missing"}}));
            }
        }
    }
    for (slot, field) in [("firstFrame","firstFrameAssetId"),("lastFrame","lastFrameAssetId")] {
        if step.kind != "video" || step.sources.iter().filter(|source| wired(source)).any(|source| source.slot == slot) { continue; }
        let requested = if slot == "lastFrame" { input["mode"] == "first-last-frame" }
            else { matches!(input["mode"].as_str(),Some("first-frame"|"first-last-frame")) };
        if requested {
            let asset = input[field].as_str().or_else(|| shot.and_then(|shot| shot[field].as_str()));
            requirements.push(json!({"slot":slot,"required":true,"assetId":asset,"satisfied":asset.is_some(),
                "state":if asset.is_some() {"bound"} else {"missing"}}));
        }
    }
    for id in waiting {
        requirements.push(json!({"slot":"after","source":id,"required":true,"satisfied":false,"state":"waiting"}));
    }
    Ok((input,pending,requirements))
}

fn diagnostic(app: &App, file: &Path, project_id: &str, project: &Value, plan: &Plan, step: &Step) -> Value {
    let (input, mut pending, mut requires) = match arguments(project_id,project,plan,step,true) {
        Ok(result) => result,
        Err(error) => return json!({"id":step.id,"state":"blocked","reason":error.payload()}),
    };
    let result = match step.kind.as_str() {
        "image" => image_queue::inspect_plan(app,&input,file,project,&pending),
        "tail" => canvas_extensions::inspect_tail_plan(app,file,project, step.node_id.as_deref().unwrap_or_default(),
            input["videoAssetId"].as_str(),!pending.is_empty()),
        "video" => {
            let takes: Vec<_> = plan.steps.iter().filter(|other| other.kind == "video" && step.after.contains(&other.id))
                .map(|other| other.input.clone()).collect();
            video_queue::inspect_plan(app,file,project,&takes,&input,&pending).and_then(|result| {
                let previous = plan.steps.iter().find(|candidate| candidate.kind == "video" &&
                    candidate.input["shotId"].is_string() && candidate.input["shotId"] == result["previousShotId"] &&
                    step.after.contains(&candidate.id));
                let planned = previous.and_then(|candidate|plan.progress.get(&candidate.id)).and_then(|progress|progress.asset_id.as_deref());
                if planned.is_some_and(|asset|result["previousAssetId"] != asset) {
                    return Err(invalid("上一镜选中的成片与本计划产物不一致。请选择本次成片，或用提取尾帧步骤连接指定版本"));
                }
                Ok(result)
            })
        }
        _ => {
            let mut draft = project.clone();
            let layout = export_layout(project,&input);
            draft["timeline"] = json!(export_clips(&layout,plan,step,&mut pending));
            for clip in draft["timeline"].as_array().into_iter().flatten().filter(|clip|clip["enabled"] != false) {
                if let Some(source) = clip["assetId"].as_str().filter(|asset|pending.iter().any(|id|id == asset))
                    .and_then(|id|id.strip_prefix("pending:")) {
                    requires.push(json!({"slot":"clipMedia","clipId":clip["id"],"shotId":clip["shotId"],
                        "source":source,"required":true,"satisfied":false,"state":"waiting"}));
                }
            }
            final_export::inspect_plan(app,file,&draft,&pending).map(|mut diagnostics| {
                diagnostics["approvalStamp"] = json!(video_queue::stamp(&json!([layout,draft["settings"],draft["bgmClip"],draft["textOverlays"]])));
                diagnostics
            })
        }
    };
    let progress = plan.progress.get(&step.id);
    let waiting = !pending.is_empty() || requires.iter().any(|item| item["state"] == "waiting");
    let state = if progress.is_some_and(|item| item.state == "succeeded") { "succeeded" }
        else if result.is_err() { "blocked" } else if waiting { "waiting" } else { "ready" };
    let assigned = result.as_ref().ok().and_then(|item| item["referenceAssignments"].as_array())
        .filter(|items| !items.is_empty());
    if let Some(assignments) = assigned {
        for assignment in assignments {
            let Some(index) = assignment["index"].as_u64().and_then(|index| index.checked_sub(1)) else { continue; };
            let slot = format!("reference:{index}");
            let mut record = requires.iter_mut().find(|item| item["slot"] == slot);
            if record.is_none() {
                requires.push(json!({"slot":slot}));
                record = requires.last_mut();
            }
            let item = record.unwrap();
            item["workflowInput"] = json!({"nodeId":assignment["nodeId"],"role":assignment["role"]});
            item["required"] = json!(assignment["required"].as_bool().unwrap_or(true));
            item["fallback"] = assignment["fallback"].clone();
            let asset = assignment["assetId"].as_str().unwrap_or_default();
            if !asset.starts_with("pending:") {
                item["assetId"] = json!(asset);
                item["satisfied"] = json!(true);
                item["state"] = json!(if assignment["fallback"] == true {"fallback"} else {"bound"});
            }
        }
    } else if step.kind == "image" {
        for (index, binding) in image_queue::plan_reference_slots(app,&input).into_iter().enumerate() {
            let slot = format!("reference:{index}");
            if let Some(requirement) = requires.iter_mut().find(|item|item["slot"] == slot) {
                requirement["workflowInput"] = binding;
            } else {
                requires.push(json!({"slot":slot,"required":true,"workflowInput":binding,"satisfied":false,"state":"missing"}));
            }
        }
    }
    for requirement in &mut requires {
        if let Some(asset_id) = requirement["assetId"].as_str() {
            let kind = if requirement["slot"] == "video" {"video"} else {"image"};
            if video_queue::media_path(app,file,asset_id,kind).is_err() {
                requirement["satisfied"] = json!(false);
                requirement["state"] = json!("missing-file");
            }
        }
    }
    let node_id = step.node_id.clone().or_else(|| match step.kind.as_str() {
        "video" => input["shotId"].as_str().map(|id| format!("shotVideo:{id}")),
        "image" => input["targetId"].as_str().map(|id| {
            let kind = match input["targetType"].as_str() {
                Some("shot-image") => "shotImage", Some("shot-board") => "shotBoard",
                Some("shot-first"|"shot-last") => "shotFrames", other => other.unwrap_or("image"),
            };
            format!("{kind}:{id}")
        }),
        "export" => Some("export".to_owned()),
        _ => None,
    });
    let label = node_id.as_deref().and_then(|id|project["canvas"]["nodes"].as_array().into_iter().flatten()
        .find(|node|node["id"] == id)).and_then(|node|node["data"]["label"].as_str())
        .or_else(||project["shots"].as_array().into_iter().flatten().find(|shot|
            shot["id"] == input["shotId"] || shot["id"] == input["targetId"]).and_then(|shot|shot["title"].as_str()));
    let reason = match &result {
        Err(error) => Some(error.payload()),
        Ok(_) if step.kind == "export" && state == "waiting" => {
            let mut sources: Vec<_> = requires.iter().filter(|item|item["state"] == "waiting")
                .filter_map(|item|item["source"].as_str()).collect();
            sources.sort_unstable();
            sources.dedup();
            Some(json!({"code":"waiting_dependency","error":format!("导出等待上游步骤：{}",sources.join("、")),"stepIds":sources}))
        }
        _ => None,
    };
    json!({"id":step.id,"kind":step.kind,"nodeId":node_id,"label":label,"state":state,"requires":requires,
        "execution":result.as_ref().ok(),"reason":reason,
        "output":progress.and_then(|item| item.asset_id.as_ref()),"progress":progress})
}

fn export_clips(layout: &Value, plan: &Plan, step: &Step, pending: &mut Vec<String>) -> Vec<Value> {
    layout.as_array().into_iter().flatten().map(|clip| {
        let mut result = clip.clone();
        if let Some(upstream) = plan.steps.iter().find(|other| other.kind == "video" &&
            other.input["shotId"] == clip["shotId"] && step.after.contains(&other.id)) {
            let asset = plan.progress.get(&upstream.id).and_then(|item|item.asset_id.clone()).unwrap_or_else(|| {
                let id = format!("pending:{}",upstream.id);
                pending.push(id.clone());
                id
            });
            result["assetId"] = json!(asset);
        }
        result
    }).collect()
}

fn export_layout(project: &Value, input: &Value) -> Value {
    if input["buildFromShots"] == true { json!(project_authoring::timeline_from_shots(project)) }
    else { input.get("clips").unwrap_or(&project["timeline"]).clone() }
}

fn approval_key(step: &Value) -> Value {
    let inputs: Vec<_> = step["requires"].as_array().into_iter().flatten().filter(|requirement|
        requirement["source"].is_null()).map(|requirement| json!([requirement["slot"],requirement["assetId"]])).collect();
    json!([step["execution"]["approvalStamp"],inputs])
}

fn approval_revision(steps: &[Value]) -> String {
    let approvals: Vec<_> = steps.iter().map(|step| json!([step["id"],step["state"],approval_key(step)])).collect();
    format!("{:016x}",video_queue::stamp(&json!(approvals)))
}

fn neighbors<'a>(edges: &'a [Value], origin: &'a str, upstream: bool) -> Vec<&'a str> {
    let (from,to) = if upstream { ("target","source") } else { ("source","target") };
    let mut seen = HashSet::from([origin]);
    let mut pending = vec![origin];
    let mut result = Vec::new();
    while let Some(node) = pending.pop() {
        for id in edges.iter().filter(|edge| edge[from] == node).filter_map(|edge|edge[to].as_str()) {
            if seen.insert(id) { pending.push(id); result.push(id); }
        }
    }
    result
}

fn binding_edges(project: &Value) -> Vec<Value> {
    let items = |key: &str| project[key].as_array().map(Vec::as_slice).unwrap_or_default();
    let nodes = project.pointer("/canvas/nodes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let stored = project.pointer("/canvas/edges").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let by_id: HashMap<_, _> = nodes.iter().filter_map(|node| Some((node["id"].as_str()?, node))).collect();
    let asset_type = |node: &Value| match node["type"].as_str()? {
        "subject" => Some("subjectId"), "scene" => Some("sceneId"), "prop" => Some("propId"), _ => None,
    };
    let shot_target = |node: &Value| node["data"]["captureId"].is_null()
        && node["data"]["shotId"].is_string() && matches!(node["type"].as_str(), Some("shotBoard" | "shotImage" | "shotFrames" | "shotVideo"));
    let mut edges: Vec<_> = stored.iter().filter(|edge| {
        let source = edge["source"].as_str().and_then(|id| by_id.get(id));
        let target = edge["target"].as_str().and_then(|id| by_id.get(id));
        !(source.is_some_and(|node| asset_type(node).is_some()) && target.is_some_and(|node| shot_target(node))
            && !matches!(edge["targetHandle"].as_str(), Some("first-frame" | "last-frame")))
    }).cloned().collect();
    let text = |shot: &Value, keys: &[&str]| keys.iter().filter_map(|key| shot[*key].as_str()).collect::<Vec<_>>().join("\n");
    for shot in items("shots") {
        let Some(shot_id) = shot["id"].as_str() else { continue };
        let targets: Vec<_> = nodes.iter().filter(|node| shot_target(node) && node["data"]["shotId"] == shot_id).collect();
        let has_stills = targets.iter().any(|node| node["type"] != "shotVideo");
        let cast_text = text(shot, &["title", "visualPrompt", "videoPrompt", "voiceover", "blocking"]);
        let stored_cast = shot["subjectIds"].as_array().map(Vec::as_slice).unwrap_or_default();
        let cast: HashSet<_> = if shot["subjectBindingMode"] == "explicit" {
            stored_cast.iter().filter_map(Value::as_str).collect()
        } else {
            let subjects = items("subjects");
            let named = stored_cast.iter().filter_map(Value::as_str).filter_map(|token| {
                let lower = token.trim().to_lowercase();
                subjects.iter().find(|subject| subject_matches(subject, |key| key.to_lowercase() == lower))
            });
            named.chain(subjects.iter().filter(|subject| subject_matches(subject, |key| cast_text.contains(key))))
                .filter_map(|subject| subject["id"].as_str()).collect()
        };
        let prop_text = text(shot, &["title", "visualPrompt", "videoPrompt", "propFocus", "blocking", "purpose"]);
        let scene_text = text(shot, &["title", "visualPrompt", "videoPrompt", "audioDescription"]);
        let scene = if shot.get("sceneId").is_some_and(Value::is_null) { None } else {
            let scenes = items("scenes");
            scenes.iter().find(|scene| scene["id"] == shot["sceneId"])
                .or_else(|| stored.iter().find_map(|edge| {
                    if matches!(edge["targetHandle"].as_str(), Some("binding" | "first-frame" | "last-frame")) { return None }
                    if !targets.iter().any(|node| node["type"] == "shotVideo" && node["id"] == edge["target"]) { return None }
                    let source = by_id.get(edge["source"].as_str()?)?;
                    if source["type"] != "scene" { return None }
                    scenes.iter().find(|scene| scene["id"] == source["data"]["sceneId"])
                }))
                .or_else(|| scenes.iter().find(|scene| scene["name"].as_str().map(str::trim)
                    .is_some_and(|name| name.chars().count() >= 2 && scene_text.contains(name))))
                .or_else(|| scenes.first())
        };
        for source in nodes {
            let Some(key) = asset_type(source) else { continue };
            let Some(id) = source["data"][key].as_str() else { continue };
            let bound = match key {
                "sceneId" => scene.is_some_and(|scene| scene["id"] == id),
                "propId" => shot["propIds"].as_array().map_or_else(|| items("props").iter().any(|prop| {
                    prop["id"] == id && prop["name"].as_str().is_some_and(|name| !name.is_empty() && prop_text.contains(name))
                }), |ids| ids.iter().any(|item| item == id)),
                "subjectId" => cast.contains(id),
                _ => false,
            };
            if !bound { continue }
            let Some(source_id) = source["id"].as_str() else { continue };
            for target in targets.iter().filter(|node| !has_stills || node["type"] != "shotVideo") {
                let Some(target_id) = target["id"].as_str() else { continue };
                edges.push(json!({"id":format!("binding:{source_id}:{target_id}"),"source":source_id,"target":target_id,
                    "sourceHandle":"out","targetHandle":"binding"}));
            }
        }
    }
    edges
}

fn subject_matches(subject: &Value, matches: impl Fn(&str) -> bool) -> bool {
    ["id", "name", "role"].iter().filter_map(|key| subject[*key].as_str()).map(str::trim)
        .filter(|value| value.encode_utf16().count() >= 2).any(matches)
}

fn graph(app: &App, file: &Path, project_id: &str, project: &Value, plan: Option<&Plan>, focus: Option<&str>) -> Result<Value> {
    let raw_nodes = project["canvas"]["nodes"].as_array().into_iter().flatten();
    let shot_labels = project_authoring::shot_node_labels(project);
    let nodes: Vec<_> = raw_nodes.map(|node| {
        let label = if matches!(node["type"].as_str(), Some("shotBoard" | "shotImage" | "shotFrames" | "shotVideo" | "audio")) {
            node["data"]["shotId"].as_str().and_then(|id| shot_labels.get(id)).map(|label| json!(label))
        } else { None };
        json!({"id":node["id"],"kind":node["type"],"label":label.unwrap_or_else(|| node["data"]["label"].clone()),
        "shotId":node["data"]["shotId"],"sceneId":node["data"]["sceneId"],"subjectId":node["data"]["subjectId"],
        "propId":node["data"]["propId"],"parentGroupId":node["parentGroupId"],"referenceSources":node["data"]["referenceSourceNodeIds"]})
    }).collect();
    let labels: HashMap<_,_> = nodes.iter().filter_map(|node| Some((node["id"].as_str()?,node["label"].clone()))).collect();
    let edges: Vec<_> = binding_edges(project).iter().map(|edge| json!({
        "id":edge["id"],"source":edge["source"],"target":edge["target"],"sourceHandle":edge["sourceHandle"],
        "targetHandle":edge["targetHandle"],"sourceLabel":edge["source"].as_str().and_then(|id|labels.get(id)),
        "targetLabel":edge["target"].as_str().and_then(|id|labels.get(id))
    })).collect();
    let execution: Vec<_> = plan.into_iter().flat_map(|plan| plan.steps.iter().map(|step| diagnostic(app,file,project_id,project,plan,step))).collect();
    let execution_edges: Vec<_> = plan.into_iter().flat_map(|plan|plan.steps.iter().flat_map(|step| {
        step.sources.iter().map(|source|json!({"source":source.step_id,"target":step.id,"targetSlot":source.slot}))
            .chain(step.after.iter().map(|id|json!({"source":id,"target":step.id,"targetSlot":"after"})))
    })).collect();
    let detail = focus.map(|id| -> Result<Value> {
        let node = nodes.iter().find(|node|node["id"] == id);
        let steps: Vec<_> = execution.iter().filter(|step|step["nodeId"] == id).collect();
        if node.is_none() && steps.is_empty() { return Err(studio_mcp::missing("找不到指定画布节点")); }
        let paths:Vec<_> = steps.iter().filter_map(|step| {
            let id = step["id"].as_str()?;
            Some(json!({"id":id,"upstream":neighbors(&execution_edges,id,true),"downstream":neighbors(&execution_edges,id,false)}))
        }).collect();
        Ok(json!({"node":node,"upstream":neighbors(&edges,id,true),"downstream":neighbors(&edges,id,false),"executionPaths":paths}))
    }).transpose()?;
    let workload = plan.map(|_| generation_review::sum_workload(execution.iter().filter(|step|
        step["state"] != "succeeded" && matches!(step["kind"].as_str(),Some("image"|"video")))
        .map(|step| &step["execution"]["workload"])));
    Ok(json!({"projectId":project_id,"updatedAt":project["updatedAt"],"nodes":nodes,"edges":edges,"executionEdges":execution_edges,
        "workload":workload,
        "planId":plan.map(|plan| &plan.id),"planVersion":plan.map(|plan|plan.version),
        "executionRevision":plan.map(|_|approval_revision(&execution)),"execution":execution,"focus":detail,
        "executionCoverage":if plan.is_some() {"planned-steps"} else {"no-plan"}}))
}

fn present(plan: &Plan, live: Option<&(String,String)>, project_id: &str) -> Value {
    let mut value = json!(plan);
    if let Some(object) = value.as_object_mut() { object.remove("approval"); object.remove("retryFailed"); }
    if plan.state == "running" && live.is_none_or(|(project,id)| project != project_id || id != &plan.id) {
        value["state"] = json!("interrupted");
    }
    value
}

fn query_job(app: &App, kind: &str, input: &Value) -> Result<Value> {
    let response = match kind {
        "image" => image_queue::call(app,"studio_image_status",input)?,
        "video" => video_queue::invoke(app,"studio_video_status",input)?,
        _ => return Ok(final_export::call(app,"studio_export_status",input)?["job"].clone()),
    };
    Ok(response[if kind == "image" {"job"} else {"batch"}].clone())
}

fn verify_capture_plan(project: &Value, plan: &Plan) -> Result<()> {
    if !plan.capture { return Ok(()); }
    let capture = &project["continuousCapture"];
    let current = capture["id"].as_str().is_some_and(|id| plan.id == id ||
        plan.id.strip_prefix(id).is_some_and(|suffix| suffix.starts_with("-scene-")));
    if project["continuousCaptureEnabled"] != true || !current ||
        !capture_plan::matches_script(project) {
        return Err(invalid("连续拍计划或剧本已更新，请重新核对"));
    }
    Ok(())
}

async fn dispatch_step(app: &App, project_id: &str, plan: &Plan, step: &Step) -> Result<Value> {
    let (file,project) = studio_mcp::project_by_id(app,project_id)?;
    let (input,_,_) = arguments(project_id,&project,plan,step,false)?;
    let request_id = input["requestId"].clone();
    let mut key = json!({"projectId":project_id,"requestId":request_id,"confirmed":true});
    let existing = if step.kind == "tail" { None } else { match query_job(app,&step.kind,&key) {
        Ok(job) => Some(job),
        Err(error) if error.0 == StatusCode::NOT_FOUND => None,
        Err(error) => return Err(error),
    }};
    if existing.as_ref().is_none_or(|job| job["status"] == "prepared") {
        verify_capture_plan(&project,plan)?;
        let current = diagnostic(app,&file,project_id,&project,plan,step);
        if current["state"] != "ready" || plan.approval.get(&step.id) != Some(&approval_key(&current)) {
            return Err(Failure::new(StatusCode::CONFLICT,"执行输入已变更，请重新核对并确认计划".into()).with_issues(vec![current]));
        }
    }
    match step.kind.as_str() {
        "image" => {
            let job = match existing { Some(job) => job,
                None => image_queue::call(app,"studio_prepare_image",&input)?["job"].clone() };
            match job["status"].as_str() {
                Some("prepared") => Ok(image_queue::call(app,"studio_confirm_image",&key)?["job"].clone()),
                Some("paused") => Ok(image_queue::call(app,"studio_resume_image",&key)?["job"].clone()),
                _ => Ok(job),
            }
        }
        "video" => {
            let job = match existing { Some(job) => job, None => {
                let mut take = input.clone();
                let object = take.as_object_mut().unwrap();
                for field in ["projectId","expectedUpdatedAt","requestId"] { object.remove(field); }
                let prep = json!({"projectId":project_id,"requestId":request_id,"expectedUpdatedAt":project["updatedAt"],"shots":[take]});
                video_queue::invoke(app,"studio_prepare_video",&prep)?["batch"].clone()
            }};
            match job["status"].as_str() {
                Some("prepared") => Ok(video_queue::invoke(app,"studio_confirm_video",&key)?["batch"].clone()),
                Some("paused") => Ok(video_queue::invoke(app,"studio_resume_video",&key)?["batch"].clone()),
                _ => Ok(job),
            }
        }
        "tail" => {
            let node = json!({"nodeId":step.node_id,"projectId":project_id,"expectedUpdatedAt":project["updatedAt"]});
            let mut result = canvas_extensions::run_tail_plan(app,&node,input["videoAssetId"].as_str(),request_id.as_str().unwrap_or_default()).await?;
            result["status"] = json!("succeeded");
            Ok(result)
        }
        _ => {
            if let Some(job) = existing { return Ok(job); }
            key["expectedUpdatedAt"] = project["updatedAt"].clone();
            if input["buildFromShots"] == true || input.get("clips").is_some() || !step.after.is_empty() {
                let layout = export_layout(&project,&input);
                let mut pending = Vec::new();
                let clips = export_clips(&layout,plan,step,&mut pending);
                if !pending.is_empty() { return Err(invalid("导出依赖的视频尚未归档")); }
                let _write = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
                let edited = project_authoring::invoke(app,"studio_edit_timeline",
                    &json!({"projectId":project_id,"expectedUpdatedAt":project["updatedAt"],"clips":clips}),None)?;
                key["expectedUpdatedAt"] = edited["updatedAt"].clone();
            }
            Ok(final_export::call(app,"studio_export_final",&key)?["job"].clone())
        }
    }
}

fn update(app: &App, path: &Path, id: &str, edit: impl FnOnce(&mut Plan)) -> Result<Plan> {
    let _lock = app.writes.lock().map_err(|_| invalid("计划写锁不可用"))?;
    let mut all = load(path)?;
    let plan = all.iter_mut().find(|plan| plan.id == id).ok_or_else(|| studio_mcp::missing("找不到计划"))?;
    edit(plan);
    let snapshot = plan.clone();
    save(path,&all)?;
    Ok(snapshot)
}

async fn execute(app: &App, project_id: &str, path: &Path, id: &str) -> Result<()> {
    loop {
        let mut plan = load(path)?.into_iter().find(|plan| plan.id == id).ok_or_else(|| studio_mcp::missing("找不到计划"))?;
        if plan.pause_requested {
            update(app,path,id,|plan| plan.state = "paused".into())?;
            return Ok(());
        }
        let Some(step) = plan.steps.iter().find(|step| plan.progress.get(&step.id).is_none_or(|item| item.state != "succeeded")).cloned() else {
            update(app,path,id,|plan| plan.state = "succeeded".into())?;
            return Ok(());
        };
        if plan.retry_failed && plan.progress.get(&step.id).is_some_and(|item|item.started) && step.kind != "tail" {
            let key = json!({"projectId":project_id,"requestId":request_id(project_id,&plan,&step)});
            if let Ok(job) = query_job(app,&step.kind,&key) {
                let retry = matches!(job["status"].as_str(),Some("failed"|"interrupted"|"discarded"|"prepared"));
                if retry {
                    if job["status"] == "prepared" {
                        if step.kind == "image" { image_queue::call(app,"studio_discard_image_plan",&key)?; }
                        if step.kind == "video" { video_queue::invoke(app,"studio_discard_video_plan",&key)?; }
                    }
                    plan = update(app,path,id,|plan| {
                        plan.progress.entry(step.id.clone()).or_default().attempt += 1;
                        plan.retry_failed = false;
                    })?;
                }
            }
        }
        let attempt = plan.progress.get(&step.id).map_or(0,|item|item.attempt);
        let generation = plan.progress.get(&step.id).and_then(|item|item.generation).or(Some(plan.version));
        update(app,path,id,|plan| {
            let item = plan.progress.entry(step.id.clone()).or_default();
            item.state = "running".into();
            item.started = true;
            item.reason = None;
            item.generation = generation;
            plan.retry_failed = false;
        })?;
        let key = json!({"projectId":project_id,"requestId":request_id(project_id,&plan,&step)});
        let mut job = dispatch_step(app,project_id,&plan,&step).await;
        while job.as_ref().is_ok_and(|job| matches!(job["status"].as_str(),Some("starting"|"running"|"submitting"|"waiting"))) {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            job = query_job(app,&step.kind,&key);
        }
        let progress = match job {
            Ok(job) if job["status"] == "succeeded" => {
                let asset = job["assetId"].as_str().or_else(||job["steps"][0]["assetId"].as_str()).or_else(||job["output"]["assetId"].as_str());
                if let Some(asset) = asset {
                    let (file,_) = studio_mcp::project_by_id(app,project_id)?;
                    video_queue::media_path(app,&file,asset,if matches!(step.kind.as_str(),"image"|"tail") {"image"} else {"video"})?;
                    Progress {state:"succeeded".into(),started:true,attempt,generation,asset_id:Some(asset.into()),reason:None}
                } else {
                    Progress {state:"blocked".into(),started:true,attempt,generation,asset_id:None,reason:Some(json!({"error":"任务缺少归档素材 ID"}))}
                }
            }
            Ok(job) => Progress { state:match job["status"].as_str() {Some("unknown")=>"unknown",Some("paused")=>"paused",_=>"blocked"}.into(),
                started:true,attempt,generation,asset_id:None,reason:Some(json!({"status":job["status"],"error":job["error"],"requestId":key["requestId"],
                    "promptId":job["promptId"].as_str().or_else(||job["steps"][0]["promptId"].as_str()),
                    "providerError":job["providerError"],"nextAction":job["nextAction"]})) },
            Err(error) => Progress {state:"blocked".into(),started:true,attempt,generation,asset_id:None,reason:Some(error.payload())},
        };
        let succeeded = progress.state == "succeeded";
        update(app,path,id,|plan| {
            if !succeeded { plan.state = progress.state.clone(); }
            plan.progress.insert(step.id.clone(),progress);
        })?;
        if !succeeded { return Ok(()); }
    }
}

pub(super) async fn browser(State(app): State<App>, Json(call): Json<Value>) -> Result<Json<Value>> {
    let name = identifier(&call,"name")?;
    let input = &call["input"];
    let result = if capture_plan::METHODS.contains(&name) { capture_plan::invoke(&app,name,input)? }
        else if name == "studio_extract_tail_frame" { canvas_write::extract_tail(&app,input).await? }
        else { call_plan(&app,name,input)? };
    Ok(Json(result))
}

pub(super) fn call_plan(app: &App, name: &str, input: &Value) -> Result<Value> {
    let project_id = identifier(input,"projectId")?;
    let mut live = app.plan_active.lock().map_err(|_| invalid("执行计划锁不可用"))?;
    let _lock = app.writes.lock().map_err(|_| invalid("计划写锁不可用"))?;
    let (file,project) = studio_mcp::project_by_id(app,project_id)?;
    let path = data_path(app,&file)?;
    let mut plans = load(&path)?;
    if name == "studio_current_plan" {
        return Ok(json!({"projectId":project_id,"updatedAt":project["updatedAt"],"plans":plans.iter().filter(|plan|
            input["includeCompleted"]==true || plan.state!="succeeded").map(|plan| present(plan,live.as_ref(),project_id)).collect::<Vec<_>>()}));
    }
    if name == "studio_canvas_graph" {
        let plan = if input.get("planId").is_some() {
            let id = identifier(input,"planId")?;
            Some(plans.iter().find(|plan| plan.id == id).ok_or_else(|| studio_mcp::missing("找不到计划"))?)
        } else { None };
        let plan = plan.or_else(|| (plans.len()==1).then(|| &plans[0]));
        return graph(app,&file,project_id,&project,plan,input.get("nodeId").map(|_|identifier(input,"nodeId")).transpose()?);
    }
    let capture_spec;
    let is_capture = name == "studio_capture_prepare";
    let (name,input) = if is_capture {
        capture_spec = capture_plan::execution_spec(app,&file,&project,input)?;
        let id = identifier(&capture_spec,"planId")?;
        if let Some(plan) = plans.iter().find(|plan| plan.id == id) {
            if !plan.capture { return Err(invalid("同名执行计划已存在")); }
            return Ok(json!({"projectId":project_id,"updatedAt":project["updatedAt"],
                "plan":present(plan,live.as_ref(),project_id),"reused":true}));
        }
        if capture_spec["steps"].as_array().is_some_and(Vec::is_empty) {
            return Ok(json!({"projectId":project_id,"updatedAt":project["updatedAt"],"completed":true,"plan":null}));
        }
        ("studio_save_plan",&capture_spec)
    } else { (name,input) };
    let id = identifier(input,"planId")?;
    let is_live = live.as_ref().is_some_and(|(project,plan)| project == project_id && plan == id);
    let index = plans.iter().position(|plan| plan.id == id);
    if name == "studio_save_plan" {
        let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
        if !is_capture { validate_project_revision(input, revision, true)?; }
        if is_live { return Err(invalid("请暂停并等待当前步骤结束后再修改计划")); }
        let steps = serde_json::from_value(input["steps"].clone()).map_err(|_| invalid("执行步骤格式无效"))?;
        let steps = validate_steps(steps)?;
        let (version,state,progress) = if let Some(idx) = index {
            let previous = &plans[idx];
            if input["expectedPlanVersion"].as_u64() != Some(previous.version) {
                return Err(Failure::new(StatusCode::CONFLICT,"计划已修改，请重新读取其版本".into()));
            }
            if previous.state == "succeeded" { return Err(invalid("已完成计划仅供查看，请另建计划")); }
            let started:Vec<_> = previous.steps.iter().filter(|step|previous.progress.get(&step.id).is_some_and(|item|item.started)).cloned().collect();
            if !steps.starts_with(&started) { return Err(invalid("已启动步骤及其顺序必须保留，只能修改后续未启动步骤")); }
            let mut progress = previous.progress.clone();
            for item in progress.values_mut().filter(|item|item.started) { item.generation.get_or_insert(previous.version); }
            (previous.version+1,if started.is_empty() {"draft".into()} else {"paused".into()},progress)
        } else { (1,"draft".into(),HashMap::new()) };
        let plan = Plan {id:id.into(),name:identifier(input,"name")?.into(),version,state,pause_requested:false,
            retry_failed:false,capture:is_capture || index.is_some_and(|idx| plans[idx].capture),
            steps,progress,approval:HashMap::new()};
        if let Some(idx) = index { plans[idx] = plan.clone(); }
        else {
            if plans.len() >= 100 { return Err(invalid("每个项目最多保存 100 份计划")); }
            plans.push(plan.clone());
        }
        save(&path,&plans)?;
        return Ok(json!({"projectId":project_id,"updatedAt":revision,"plan":plan}));
    }
    let idx = index.ok_or_else(|| studio_mcp::missing("找不到计划"))?;
    match name {
        "studio_pause_plan" => {
            if !is_live { return Err(invalid("此计划未在运行")); }
            plans[idx].pause_requested = true;
            save(&path,&plans)?;
        }
        "studio_run_plan" => {
            if is_live && plans[idx].state == "running" || plans[idx].state == "succeeded" {
                return Ok(json!({"projectId":project_id,"plan":present(&plans[idx],live.as_ref(),project_id),"reused":true}));
            }
            verify_capture_plan(&project,&plans[idx])?;
            if input["confirmed"] != true { return Err(invalid("请先核对全图并确认运行计划")); }
            if live.is_some() { return Err(Failure::new(StatusCode::LOCKED,"已有执行计划运行".into())); }
            let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
            validate_project_revision(input, revision, false)?;
            if input["expectedPlanVersion"].as_u64()!=Some(plans[idx].version) { return Err(invalid("计划已修改，请重新核对全图")); }
            let checks: Vec<_> = plans[idx].steps.iter().map(|step| diagnostic(app,&file,project_id,&project,&plans[idx],step)).collect();
            if input["expectedExecutionRevision"].as_str() != Some(approval_revision(&checks).as_str()) {
                return Err(Failure::new(StatusCode::CONFLICT,"执行配置或依赖已变化，请重新核对全图".into()));
            }
            let mut approval = HashMap::new();
            for (step,diagnostic) in plans[idx].steps.iter().zip(checks) {
                if diagnostic["state"] == "blocked" && plans[idx].progress.get(&step.id).is_none_or(|item| !item.started) {
                    return Err(invalid("计划有阻塞项，请查看全图中的执行诊断").with_issues(vec![diagnostic]));
                }
                approval.insert(step.id.clone(),approval_key(&diagnostic));
            }
            plans[idx].approval = approval;
            plans[idx].state = "running".into();
            plans[idx].pause_requested = false;
            plans[idx].retry_failed = input["retryFailed"] == true;
            save(&path,&plans)?;
            *live = Some((project_id.into(),id.into()));
            let app = app.clone();
            let project_id = project_id.to_owned();
            let id = id.to_owned();
            tokio::spawn(async move {
                if let Err(error) = execute(&app,&project_id,&path,&id).await {
                    let _ = update(&app,&path,&id,|plan| {
                        plan.state = "blocked".into();
                        if let Some(step) = plan.steps.iter().find(|step| plan.progress.get(&step.id).is_none_or(|item| item.state != "succeeded")) {
                            let item = plan.progress.entry(step.id.clone()).or_default();
                            item.state = "blocked".into();
                            item.reason = Some(error.payload());
                        }
                    });
                }
                if let Ok(mut live) = app.plan_active.lock() { *live = None; }
            });
        }
        _ => return Err(studio_mcp::missing("未知计划操作")),
    }
    Ok(json!({"projectId":project_id,"plan":present(&plans[idx],live.as_ref(),project_id)}))
}
