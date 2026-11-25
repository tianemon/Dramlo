use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const METHODS: &[&str] = &[
    "studio_capture_state",
    "studio_capture_set_plan",
    "studio_capture_approve",
];

/// 连续拍保存原始计划和画布。排队、确认、暂停与恢复使用统一执行计划。
const FREE_KINDS: &[&str] = &["character", "prop", "scene"];

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn current_project(app: &App, input: &Value) -> Result<(PathBuf, Value)> {
    if let Some(id) = input["projectId"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
    {
        return studio_mcp::project_by_id(app, id);
    }
    studio_mcp::active_project(app)?.ok_or_else(|| invalid("当前没有打开的项目"))
}

fn save_project(path: &Path, value: &Value) -> Result<()> {
    atomic_save(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| invalid("无法保存项目"))?,
    )
}

fn text(value: &Value, key: &str, limit: usize) -> Result<String> {
    let body = value[key]
        .as_str()
        .map(str::trim)
        .filter(|body| !body.is_empty())
        .ok_or_else(|| invalid(&format!("计划缺少 {key}")))?;
    if body.encode_utf16().count() > limit {
        return Err(invalid(&format!("{key} 过长")));
    }
    Ok(body.to_owned())
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| invalid(&format!("{key} 不能为空")))
}

/// 只校验结构，不评价创作质量——内容由调用方负责。
fn validate_plan(raw: &Value) -> Result<()> {
    text(raw, "title", 120)?;
    text(raw, "aesthetic", 400)?;
    let scenes = list(raw,"scenes")?;
    if scenes.len() > 100 || raw["assets"].as_array().is_some_and(|items| items.len() > 100) {
        return Err(invalid("计划最多可包含 100 场和 100 个参考资产"));
    }
    let mut asset_ids = std::collections::HashSet::new();
    for asset in raw["assets"].as_array().ok_or_else(|| invalid("assets 须为数组"))? {
        let id = text(asset, "id", 80)?;
        if asset["id"] != id || !asset_ids.insert(id.clone()) {
            return Err(invalid(&format!("资产 id 重复：{id}")));
        }
        text(asset, "name", 120)?;
        let kind = text(asset, "kind", 20)?;
        if !FREE_KINDS.contains(&kind.as_str()) {
            return Err(invalid("资产 kind 只能是 character、prop 或 scene"));
        }
        text(asset, "prompt", 8000)?;
    }
    let mut scene_ids = std::collections::HashSet::new();
    for scene in scenes {
        let id = text(scene, "id", 80)?;
        if scene["id"] != id || !scene_ids.insert(id.clone()) {
            return Err(invalid(&format!("场景 ID 重复或包含首尾空白：{id}")));
        }
        text(scene, "name", 120)?;
        text(scene, "sourceText", 8000)?;
        text(scene, "openingPrompt", 8000)?;
        for id in scene["assetIds"].as_array().ok_or_else(|| invalid("assetIds 须为数组"))? {
            let value = id.as_str().unwrap_or("");
            if !asset_ids.contains(value) {
                return Err(invalid(&format!("场景引用了不存在的资产：{value}")));
            }
        }
        let segments = list(scene, "segments")?;
        if segments.len() > 100 { return Err(invalid("每场最多 100 个子镜头")); }
        for segment in segments {
            text(segment, "title", 120)?;
            text(segment, "prompt", 8000)?;
            text(segment, "startState", 2000)?;
            text(segment, "endState", 2000)?;
            text(segment, "sound", 2000)?;
            let duration = segment["duration"].as_f64().unwrap_or(0.0);
            if !(6.0..=30.0).contains(&duration) {
                return Err(invalid("每个子镜头时长须在 6 到 30 秒之间"));
            }
            for id in segment["referenceAssetIds"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let value = id.as_str().unwrap_or("");
                if !asset_ids.contains(value) {
                    return Err(invalid(&format!("镜头引用了不存在的资产：{value}")));
                }
            }
            for line in segment["dialogue"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                text(line, "speaker", 80)?;
                text(line, "text", 2000)?;
                text(line, "delivery", 200)?;
            }
        }
    }
    Ok(())
}

pub(super) fn matches_script(project: &Value) -> bool {
    let capture = &project["continuousCapture"];
    capture["sourceFromCanvas"] != true ||
        capture["source"].as_str().map(str::trim) == project["script"]["content"].as_str().map(str::trim)
}

fn segment_text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    video_queue::field(value,key).map_err(|_| invalid(&format!("连续拍缺少 {key}")))
}

fn usable_output<'a>(app: &App, file: &Path, project: &'a Value, node_id: &str, media: &str) -> Option<&'a str> {
    let node = project["canvas"]["nodes"].as_array()?.iter().find(|node| node["id"] == node_id)?;
    let id = node["data"]["outputAssetId"].as_str()?;
    video_queue::media_path(app,file,id,media).ok()?;
    Some(id)
}

fn video_scene_ready(app: &App, file: &Path, project: &Value, capture: &Value, scene: &Value) -> bool {
    let Some(id) = scene["id"].as_str() else { return false };
    scene["segments"].as_array().is_some_and(|segments| !segments.is_empty() &&
        segments.iter().enumerate().all(|(at,_)| capture["videoNodes"][format!("{id}:{at}")].as_str()
            .is_some_and(|node| usable_output(app,file,project,node,"video").is_some())))
}

fn scene_images_complete(scene: &Value) -> bool {
    scene["openingReady"] == true && scene["assets"].as_array().into_iter().flatten().all(|item|item["ready"] == true)
}

fn scene_complete(scene: &Value) -> bool {
    scene_images_complete(scene) && scene["segments"].as_array().into_iter().flatten().all(|item|item["ready"] == true)
}

fn image_step(node: &str, sources: Vec<Value>, refs: Vec<Value>) -> Value {
    json!({"id":node,"kind":"image","nodeId":node,"sources":sources,
        "input":{"targetType":"canvas-node","targetId":node,"referenceAssetIds":refs}})
}

/// Materialize native queue steps from the persisted capture graph. This only
/// creates a draft; approval and worker progress use the standard plan API.
pub(super) fn execution_spec(app: &App, file: &Path, project: &Value, input: &Value) -> Result<Value> {
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input,revision,true)?;
    let capture = &project["continuousCapture"];
    let capture_id = segment_text(capture,"id")?;
    if project["continuousCaptureEnabled"] != true || input["captureId"] != capture_id {
        return Err(invalid("当前连续拍计划已变更，请重新读取"));
    }
    if !matches_script(project) {
        return Err(invalid("剧本已更新，请先更新连续拍计划"));
    }
    validate_plan(&capture["plan"])?;
    let scenes = list(&capture["plan"],"scenes")?;
    let scene_range = if capture["approvalMode"] == "scene" {
        let at = input["sceneIndex"].as_u64().and_then(|at|usize::try_from(at).ok())
            .filter(|at| *at < scenes.len()).ok_or_else(|| invalid("逐场模式须填写有效的 sceneIndex"))?;
        if !scenes[..at].iter().all(|scene| video_scene_ready(app,file,project,capture,scene)) {
            return Err(invalid("先完成前面的场次，再准备本场"));
        }
        at..at+1
    } else {
        if input.get("sceneIndex").is_some() { return Err(invalid("整批模式不需要 sceneIndex")); }
        0..scenes.len()
    };
    let id = if capture["approvalMode"] == "scene" {
        format!("{capture_id}-scene-{}",scene_range.start)
    } else { capture_id.to_owned() };
    let default_profile = segment_text(capture,"profileId")?;
    let nodes = project["canvas"]["nodes"].as_array().ok_or_else(||invalid("缺少连续拍画布"))?;
    let mut steps = Vec::new();
    let mut scheduled = std::collections::HashSet::new();
    let references = |ids: &[Value]| -> Result<(Vec<Value>,Vec<Value>)> {
        if ids.len() > 10 { return Err(invalid("每个输入最多绑定 10 张参考图")); }
        let mut refs = Vec::new();
        let mut sources = Vec::new();
        for (i,asset) in ids.iter().enumerate() {
            let node = segment_text(&capture["assetNodes"],asset.as_str().unwrap_or_default())?;
            if let Some(id) = usable_output(app,file,project,node,"image") {
                refs.push(json!(id));
            } else {
                refs.push(Value::Null);
                sources.push(json!({"stepId":node,"slot":format!("reference:{i}")}));
            }
        }
        Ok((refs,sources))
    };
    for scene in &scenes[scene_range.clone()] {
        let scene_id = segment_text(scene,"id")?;
        let segments = list(scene,"segments")?;
        let asset_ids = scene["assetIds"].as_array().map(Vec::as_slice).unwrap_or_default();
        for asset in asset_ids.iter().chain(segments.iter().flat_map(|segment|segment["referenceAssetIds"].as_array().into_iter().flatten())) {
            let node = segment_text(&capture["assetNodes"],asset.as_str().unwrap_or_default())?;
            if usable_output(app,file,project,node,"image").is_none() && scheduled.insert(node) {
                steps.push(image_step(node,vec![],vec![]));
            }
        }
        let opening = segment_text(&capture["openingNodes"],scene_id)?;
        let opening_asset = usable_output(app,file,project,opening,"image");
        if opening_asset.is_none() {
            let (refs,sources) = references(asset_ids)?;
            steps.push(image_step(opening,sources,refs));
        }
        let mut previous = None;
        for (at,segment) in segments.iter().enumerate() {
            let node_id = segment_text(&capture["videoNodes"],&format!("{scene_id}:{at}"))?;
            let prior = previous.replace(node_id);
            if usable_output(app,file,project,node_id,"video").is_some() { continue; }
            let node = nodes.iter().find(|node| node["id"] == node_id && node["type"] == "videoGen"
                && node["data"]["captureId"] == capture_id).ok_or_else(|| invalid("连续拍视频节点不存在"))?;
            let profile = video_queue::get_profile(app,Some(node["data"]["videoProfileId"].as_str().unwrap_or(default_profile)))?;
            video_queue::endpoint(&profile)?;
            if video_queue::frame_capabilities(&profile) != Some((true,false)) {
                return Err(invalid("连续拍需要支持单首帧的 ComfyUI 工作流"));
            }
            // Profile choice, node prompt and duration are read by the worker's
            // diagnostics, so a change after approval requires re-confirmation.
            let (refs,mut sources) = references(segment["referenceAssetIds"].as_array().map(Vec::as_slice).unwrap_or_default())?;
            let after: Vec<_> = prior.filter(|id| scheduled.contains(id)).into_iter().collect();
            let mut take = json!({"shotId":node_id,"mode":if at == 0 {"first-frame"} else {"previous-tail"},"referenceAssetIds":refs});
            if at == 0 {
                if let Some(asset) = opening_asset { take["firstFrameAssetId"] = json!(asset); }
                else { sources.push(json!({"stepId":opening,"slot":"firstFrame"})); }
            }
            steps.push(json!({"id":node_id,"kind":"video","nodeId":node_id,"input":take,"sources":sources,"after":after}));
            scheduled.insert(node_id);
        }
    }
    let title = &capture["plan"]["title"];
    let name = if capture["approvalMode"] == "scene" {
        format!("连续拍 · {}", scenes[scene_range.start]["name"].as_str().unwrap_or_default())
    } else { format!("连续拍 · {}",title.as_str().unwrap_or_default()) };
    Ok(json!({"projectId":project["id"],"planId":id,"name":name.chars().take(150).collect::<String>(),"steps":steps}))
}

fn node_map(capture: &Value, key: &str) -> Value {
    capture[key].clone()
}

/// 返回（已产出，已提交未完成）。第二项对视频重要：有任务 ID 但还没出
/// 结果时，重复提交会重复计费。
fn node_flags(app: &App, file: &Path, project: &Value, node_id: &str) -> (bool, bool) {
    project["canvas"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| node["id"].as_str() == Some(node_id))
        .map(|node| {
            let media = if node["type"] == "videoGen" { "video" } else { "image" };
            let ready = usable_output(app,file,project,node_id,media).is_some();
            let pending = node["data"]["videoTaskId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
                || node["data"]["captureSubmissionPending"] == true;
            (ready, pending)
        })
        .unwrap_or((false, false))
}

/// 读计划、阶段和每项产物的完成情况。
fn state(app: &App, input: &Value) -> Result<Value> {
    let (file, project) = current_project(app, input)?;
    let capture = &project["continuousCapture"];
    if capture.is_null() {
        return Ok(json!({
            "enabled": project["continuousCaptureEnabled"] == true,
            "capture": Value::Null
        }));
    }
    let asset_nodes = node_map(capture, "assetNodes");
    let opening_nodes = node_map(capture, "openingNodes");
    let video_nodes = node_map(capture, "videoNodes");

    let mut scenes = Vec::new();
    for (index, scene) in capture["plan"]["scenes"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let scene_id = scene["id"].as_str().unwrap_or("");
        let assets: Vec<Value> = scene["assetIds"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|id| {
                let node_id = asset_nodes[id.as_str().unwrap_or("")].as_str().unwrap_or("");
                let (ready, pending) = node_flags(app,&file,&project, node_id);
                json!({ "assetId": id, "nodeId": node_id, "ready": ready, "pending": pending })
            })
            .collect();
        let opening_node = opening_nodes[scene_id].as_str().unwrap_or("");
        let segments: Vec<Value> = scene["segments"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(at, segment)| {
                let key = format!("{scene_id}:{at}");
                let node_id = video_nodes[key.as_str()].as_str().unwrap_or("");
                let (ready, pending) = node_flags(app,&file,&project, node_id);
                json!({
                    "index": at, "title": segment["title"], "duration": segment["duration"],
                    "nodeId": node_id, "ready": ready, "pending": pending
                })
            })
            .collect();
        let (opening_ready, opening_pending) = node_flags(app,&file,&project, opening_node);
        scenes.push(json!({
            "index": index, "sceneId": scene_id, "name": scene["name"],
            "approved": capture["approvedScenes"].as_array().into_iter().flatten()
                .any(|id| id.as_str() == Some(scene_id)),
            "assets": assets, "openingNodeId": opening_node,
            "openingReady": opening_ready, "openingPending": opening_pending,
            "segments": segments
        }));
    }

    let phase = if scenes.iter().all(scene_complete) { "complete" }
       else if scenes.iter().all(scene_images_complete) { "videos" } else { "assets" };
    Ok(json!({
        "enabled": project["continuousCaptureEnabled"] == true,
        "capture": {
            "id": capture["id"],
            "phase": phase,
            "sceneIndex": capture["sceneIndex"],
            "nextSceneIndex": scenes.iter().position(|scene| !scene_complete(scene)),
            "approvalMode": capture["approvalMode"].as_str().unwrap_or("batch"),
            "videosApproved": capture["videosApproved"],
            "sourceFromCanvas": capture["sourceFromCanvas"],
            "externallyAuthored": capture["externallyAuthored"],
            "aspectRatio": capture["aspectRatio"],
            "model": capture["model"],
            "profileId": capture["profileId"],
            "maxSeconds": capture["maxSeconds"],
            "plan": capture["plan"],
            "scenes": scenes
        }
    }))
}

fn push_node(canvas: &mut Value, node: Value) -> Result<()> {
    canvas["nodes"]
        .as_array_mut()
        .ok_or_else(|| invalid("项目画布格式无效"))?
        .push(node);
    Ok(())
}

fn push_edge(canvas: &mut Value, source: &str, target: &str) -> Result<()> {
    let edges = canvas["edges"]
        .as_array_mut()
        .ok_or_else(|| invalid("项目画布格式无效"))?;
    if edges.iter().any(|edge| {
        edge["source"].as_str() == Some(source) && edge["target"].as_str() == Some(target)
    }) {
        return Ok(());
    }
    edges.push(json!({
        "id": format!("{}--{}", source, target),
        "source": source, "target": target,
        "sourceHandle": "out", "targetHandle": "in"
    }));
    Ok(())
}

/// 写入计划并建好画布节点。同一项目重复调用会替换旧计划。
fn set_plan(app: &App, input: &Value) -> Result<Value> {
    let (file, mut project) = current_project(app, input)?;
    let revision = project["updatedAt"]
        .as_u64()
        .ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;
    let approval_mode = input["approvalMode"].as_str().unwrap_or("batch");
    if !matches!(approval_mode,"batch"|"scene") {
        return Err(invalid("approvalMode 须为 batch 或 scene"));
    }
    let plan = input.get("plan").ok_or_else(|| invalid("缺少 plan"))?;
    validate_plan(plan)?;


    // 替换计划时先清掉旧计划的节点和悬空连线，避免画布上留两套。
    let mut removed_nodes = 0_usize;
    if let Some(old) = project["continuousCapture"]["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
    {
        let busy = project["canvas"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|node| node["data"]["captureId"].as_str() == Some(old.as_str()))
            .find(|node| {
                node["data"]["videoTaskId"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
                    || node["data"]["captureSubmissionPending"] == true
            });
        if let Some(node) = busy {
            let label = node["data"]["label"].as_str().unwrap_or("某个镜头");
            return Err(invalid(&format!(
                "旧计划的「{label}」还有已提交、未确认结果的视频任务。请先核对任务状态，再替换计划，避免重复计费"
            )));
        }
        if let Some(nodes) = project["canvas"]["nodes"].as_array_mut() {
            let before = nodes.len();
            nodes.retain(|node| node["data"]["captureId"].as_str() != Some(old.as_str()));
            removed_nodes = before - nodes.len();
        }
        let known: std::collections::HashSet<String> = project["canvas"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|node| node["id"].as_str().map(str::to_owned))
            .collect();
        if let Some(edges) = project["canvas"]["edges"].as_array_mut() {
            edges.retain(|edge| {
                edge["source"]
                    .as_str()
                    .is_some_and(|id| known.contains(id))
                    && edge["target"]
                        .as_str()
                        .is_some_and(|id| known.contains(id))
            });
        }
    }

    // 借用前先取出画幅与默认视频配置。
    let aspect = project["settings"]["aspectRatio"]
        .as_str()
        .unwrap_or("16:9")
        .to_string();
    let resolution = project["settings"]["defaultVideoResolution"].clone();
    let video = studio_mcp::default_model(app, "video").unwrap_or(Value::Null);
    if video["id"].as_str().is_none_or(str::is_empty) {
        return Err(invalid("请先在设置里配置并启用一个视频模型"));
    }
    let profile_id = video["id"].as_str().unwrap_or("").to_string();
    let profile = video_queue::get_profile(app, Some(&profile_id))?;
    let model = profile["workflowId"].as_str().unwrap_or("").to_string();
    let max_seconds = profile["maxSeconds"].as_f64().unwrap_or(10.0);

    let capture_id = format!("capture-{}", now().max(revision.saturating_add(1)));
    let source = input["source"].as_str().unwrap_or("").trim().to_string();
    let mut asset_nodes = serde_json::Map::new();
    let mut opening_nodes = serde_json::Map::new();
    let mut video_nodes = serde_json::Map::new();

    let origin_x = project["canvas"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let x = node["position"]["x"].as_f64()?;
            let width = node["data"]["width"].as_f64().unwrap_or(300.0);
            Some(x + width)
        })
        .fold(0.0_f64, f64::max)
        + 100.0;
    let plan_width = 440.0;
    let plan_height = 720.0;
    let scene_top = plan_height + 80.0;
    let caption = |scene_id: &str, at: usize, segment: &Value| -> String {
        let dialogue = segment["dialogue"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|line| {
                format!(
                    "{}（{}）：{}",
                    line["speaker"].as_str().unwrap_or(""),
                    line["delivery"].as_str().unwrap_or(""),
                    line["text"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "{}\n场景{}-{} · {}秒\n起幅：{}\n{}\n{}\n环境音：{}\n落幅：{}",
            plan["aesthetic"].as_str().unwrap_or(""),
            at + 1,
            scene_ordinal(plan, scene_id),
            segment["duration"].as_f64().unwrap_or(6.0),
            segment["startState"].as_str().unwrap_or(""),
            segment["prompt"].as_str().unwrap_or(""),
            dialogue,
            segment["sound"].as_str().unwrap_or(""),
            segment["endState"].as_str().unwrap_or("")
        )
    };

    let canvas = project["canvas"].clone();
    let mut canvas = canvas;
    push_node(
        &mut canvas,
        json!({
            "id": format!("{}-plan", capture_id), "type": "captureText",
            "position": { "x": origin_x, "y": 0.0 },
            "data": { "label": "连续拍 · 计划", "prompt": "", "captureId": capture_id,
                      "width": plan_width, "height": plan_height }
        }),
    )?;

    for (at, asset) in plan["assets"].as_array().into_iter().flatten().enumerate() {
        let node_id = format!("{}-asset-{}", capture_id, at);
        if let Some(asset_id) = asset["id"].as_str() {
            asset_nodes.insert(asset_id.to_owned(), json!(node_id));
        }
        push_node(
            &mut canvas,
            json!({
                "id": node_id, "type": "imageGen",
                "position": { "x": origin_x + plan_width + 80.0 + at as f64 * 300.0, "y": 0.0 },
                "status": "idle",
                "data": { "label": asset["name"], "prompt": asset["prompt"],
                          "captureId": capture_id, "aspectRatio": "16:9" }
            }),
        )?;
    }

    for (scene_at, scene) in plan["scenes"].as_array().into_iter().flatten().enumerate() {
        let scene_id = scene["id"].as_str().unwrap_or("");
        let opening = format!("{}-opening-{}", capture_id, scene_at);
        opening_nodes.insert(scene_id.to_owned(), json!(opening));
        push_node(
            &mut canvas,
            json!({
                "id": opening, "type": "imageGen",
                "position": { "x": origin_x, "y": scene_top + scene_at as f64 * 700.0 },
                "status": "idle",
                "data": { "label": format!("{} · 起始画面", scene["name"].as_str().unwrap_or("")),
                          "prompt": scene["openingPrompt"], "captureId": capture_id,
                          "aspectRatio": aspect }
            }),
        )?;
        for asset_id in scene["assetIds"].as_array().into_iter().flatten() {
            if let Some(node_id) = asset_nodes[asset_id.as_str().unwrap_or("")].as_str() {
                push_edge(&mut canvas, node_id, &opening)?;
            }
        }
        let mut previous: Option<String> = None;
        for (at, segment) in scene["segments"].as_array().into_iter().flatten().enumerate() {
            let video_id = format!("{}-video-{}-{}", capture_id, scene_at, at);
            video_nodes.insert(format!("{}:{}", scene_id, at), json!(video_id));
            let text_id = format!("{}-text", video_id);
            push_node(
                &mut canvas,
                json!({
                    "id": text_id, "type": "captureText",
                    "position": {
                        "x": origin_x + plan_width + 80.0 + at as f64 * 400.0,
                        "y": scene_top + scene_at as f64 * 700.0
                    },
                    "data": { "label": format!("场景{}-{} · 分镜稿", scene_at + 1, at + 1),
                              "prompt": caption(scene_id, at, segment),
                              "captureId": capture_id }
                }),
            )?;
            push_node(
                &mut canvas,
                json!({
                    "id": video_id, "type": "videoGen",
                    "position": {
                        "x": origin_x + plan_width + 80.0 + at as f64 * 400.0,
                        "y": scene_top + 390.0 + scene_at as f64 * 700.0
                    },
                    "status": "idle",
                    "data": {
                        "label": format!("场景{}-{}", scene_at + 1, at + 1),
                        "prompt": segment["prompt"], "captureId": capture_id,
                        "duration": segment["duration"], "videoModel": model,
                        "videoProfileId": profile_id, "aspectRatio": aspect,
                        "videoResolution": resolution
                    }
                }),
            )?;
            push_edge(&mut canvas, &text_id, &video_id)?;
            for asset in segment["referenceAssetIds"].as_array().into_iter().flatten() {
                if let Some(node) = asset_nodes[asset.as_str().unwrap_or_default()].as_str() {
                    push_edge(&mut canvas,node,&video_id)?;
                }
            }
            let upstream = previous.clone().unwrap_or_else(|| opening.clone());
            push_edge(&mut canvas, &upstream, &video_id)?;
            previous = Some(video_id);
        }
    }
    project["canvas"] = canvas;

    let capture = json!({
        "id": capture_id,
        "source": source,
        "sourceFromCanvas": !source.is_empty() && source == project["script"]["content"].as_str().unwrap_or("").trim(),
        "externallyAuthored": true,
        "approvalMode": approval_mode,
        "plan": plan,
        "model": model,
        "profileId": profile_id,
        "modelName": video["name"],
        "maxSeconds": max_seconds,
        "aspectRatio": aspect,
        "phase": "assets",
        "sceneIndex": 0,
        "approvedScenes": [],
        "videosApproved": false,
        "assetNodes": asset_nodes,
        "openingNodes": opening_nodes,
        "videoNodes": video_nodes,
        "tailAssets": {}
    });
    project["continuousCaptureEnabled"] = json!(true);
    project["continuousCapture"] = capture.clone();

    let revision = now().max(revision.saturating_add(1));
    project["updatedAt"] = json!(revision);
    save_project(&file, &project)?;
    notify_project_revision(app, json!({ "projectId": project["id"], "updatedAt": revision }));
    Ok(json!({
        "projectId": project["id"], "captureId": capture["id"], "phase": "assets",
        "approvalMode":approval_mode,
        "scenes": plan["scenes"].as_array().map(Vec::len).unwrap_or(0),
        "assets": plan["assets"].as_array().map(Vec::len).unwrap_or(0),
        "removedNodes": removed_nodes,
        "updatedAt": revision
    }))
}

fn scene_ordinal(plan: &Value, scene_id: &str) -> usize {
    plan["scenes"]
        .as_array()
        .into_iter()
        .flatten()
        .position(|scene| scene["id"].as_str() == Some(scene_id))
        .map(|index| index + 1)
        .unwrap_or(1)
}

/// 标记阶段已批准并推进游标。不触发生成，只改状态。
fn approve(app: &App, input: &Value) -> Result<Value> {
    let (file, mut project) = current_project(app, input)?;
    let revision = project["updatedAt"]
        .as_u64()
        .ok_or_else(|| invalid("项目修订号无效"))?;

    let capture_id = input["captureId"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| invalid("缺少 captureId"))?;
    let phase = input["phase"]
        .as_str()
        .filter(|phase| matches!(*phase, "assets" | "videos"))
        .ok_or_else(|| invalid("phase 只能是 assets 或 videos"))?;
    if project["continuousCapture"]["id"].as_str() != Some(capture_id) {
        return Err(invalid("连续拍计划已变更，请重新读取"));
    }
    let capture = &project["continuousCapture"];
    let index = capture["sceneIndex"].as_u64().unwrap_or(0);
    let scenes = capture["plan"]["scenes"].as_array().ok_or_else(|| invalid("连续拍场次无效"))?;
    if phase == "assets" {
        let expected = input["sceneIndex"].as_u64().ok_or_else(|| invalid("批准场次必须填写 sceneIndex"))?;
        if expected < index && scenes.get(expected as usize).is_some_and(|scene|
            capture["approvedScenes"].as_array().into_iter().flatten().any(|id| *id == scene["id"])) {
            return Ok(json!({"projectId":project["id"],"captureId":capture_id,"phase":capture["phase"],
                "sceneIndex":index,"updatedAt":revision,"reused":true}));
        }
        if expected != index { return Err(invalid("连续拍场次已变更，请重新读取")); }
        let scene = scenes.get(index as usize).ok_or_else(|| invalid("没有待准备的场景"))?;
        let id = segment_text(scene,"id")?;
        let ready = usable_output(app,&file,&project,segment_text(&capture["openingNodes"],id)?,"image").is_some() &&
            scene["assetIds"].as_array().into_iter().flatten().all(|asset| asset.as_str().is_some_and(|asset|
                capture["assetNodes"][asset].as_str().is_some_and(|node| usable_output(app,&file,&project,node,"image").is_some())));
        if !ready { return Err(invalid("本场参考资产或开场图尚未归档，无法标记为完成")); }
    } else {
        if capture["videosApproved"] == true && capture["phase"] == "complete" {
            return Ok(json!({"projectId":project["id"],"captureId":capture_id,"phase":"complete",
                "sceneIndex":index,"updatedAt":revision,"reused":true}));
        }
        if !scenes.iter().all(|scene| video_scene_ready(app,&file,&project,capture,scene)) {
            return Err(invalid("仍有视频未归档，无法标记连续拍完成"));
        }
    }
    validate_project_revision(input, revision, true)?;
    let current = project["continuousCapture"]["phase"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    if current != phase {
        return Err(invalid(&format!(
            "当前阶段是 {}，不能批准 {}",
            if current.is_empty() { "未知" } else { &current },
            phase
        )));
    }

    let next: Value;
    let mut scene_index = project["continuousCapture"]["sceneIndex"]
        .as_u64()
        .unwrap_or(0) as usize;
    {
        let capture = project["continuousCapture"]
            .as_object_mut()
            .ok_or_else(|| invalid("连续拍状态无效"))?;
        if phase == "assets" {
            let scenes = capture["plan"]["scenes"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let scene = scenes
                .get(scene_index)
                .ok_or_else(|| invalid("没有待准备的场景"))?
                .clone();
            let scene_id = scene["id"].as_str().unwrap_or("");
            let approved = capture["approvedScenes"]
                .as_array_mut()
                .ok_or_else(|| invalid("连续拍状态无效"))?;
            if !approved.iter().any(|id| id.as_str() == Some(scene_id)) {
                approved.push(json!(scene_id));
            }
            scene_index += 1;
            capture["sceneIndex"] = json!(scene_index);
            if scene_index >= scenes.len() {
                capture["phase"] = json!("videos");
            }
            next = json!({
                "sceneId": scene_id, "sceneName": scene["name"],
                "assetNodeIds": scene["assetIds"].as_array().into_iter().flatten()
                    .filter_map(|id| capture["assetNodes"][id.as_str()?].as_str().map(str::to_owned))
                    .collect::<Vec<_>>(),
                "openingNodeId": capture["openingNodes"][scene_id]
            });
        } else {
            capture["videosApproved"] = json!(true);
            capture["phase"] = json!("complete");
            next = json!({ "videosApproved": true });
        }
    }

    let phase_after = project["continuousCapture"]["phase"].clone();
    let revision = now().max(revision.saturating_add(1));
    project["updatedAt"] = json!(revision);
    save_project(&file, &project)?;
    notify_project_revision(app, json!({ "projectId": project["id"], "updatedAt": revision }));
    Ok(json!({
        "projectId": project["id"], "captureId": capture_id,
        "phase": phase_after, "sceneIndex": scene_index,
        "next": next, "updatedAt": revision
    }))
}

pub(super) fn invoke(app: &App, name: &str, input: &Value) -> Result<Value> {
    let plan_guard = app.plan_active.lock().map_err(|_| invalid("执行计划锁不可用"))?;
    let video_guard = app.video_active.lock().map_err(|_| invalid("视频队列锁不可用"))?;
    let _write = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    if name == "studio_capture_set_plan" {
        let (_, project) = current_project(app, input)?;
        let id = segment_text(&project,"id")?;
        if plan_guard.as_ref().is_some_and(|(project,_)| project == id) ||
            video_queue::shot_change_context(app,id,video_guard.as_ref())?["blocked"] == true {
            return Err(invalid("此项目仍有运行或待核对的任务，请处理完再替换连续拍计划"));
        }
    }
    match name {
        "studio_capture_state" => state(app, input),
        "studio_capture_set_plan" => set_plan(app, input),
        "studio_capture_approve" => approve(app, input),
        _ => Err(invalid("未知的连续拍操作")),
    }
}
