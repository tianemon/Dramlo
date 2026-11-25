use super::*;
use axum::extract::{State, rejection::JsonRejection};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const METHODS: &[&str] = &[
    "studio_project",
    "studio_list_projects",
    "studio_open_project",
    "studio_read_script",
    "studio_shots",
    "studio_project_overview",
    "studio_crop_shot_frame",
    "studio_project_items",
    "studio_set_video_prompt",
    "studio_locate_asset",
    "studio_save_exit",
    "studio_list_models",
    "studio_describe_model",
    "studio_list_skills",
    "studio_get_guide",
    "studio_read_skill",
    "studio_add_model",
    "studio_select_model",
    "studio_model_update",
    "studio_remove_model",
    "studio_set_credential",
    "studio_patch_workflow",
    "studio_delete_project",
    "studio_remove_project_asset",
];

#[derive(Deserialize)]
pub(super) struct Call {
    name: String,
    input: Value,
}

pub(super) fn missing(message: &str) -> Failure {
    Failure::new(StatusCode::NOT_FOUND, message.into())
}

pub(super) fn read_json(file: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(file)?).map_err(|_| invalid("项目数据无效"))
}

pub(super) fn validate_project_document(project: &Value) -> Result<()> {
    if project["id"].as_str().is_none_or(str::is_empty) || !project["name"].is_string() ||
        !project["createdAt"].is_number() || !project["updatedAt"].is_number() ||
        !project["shots"].is_array() || !project["timeline"].is_array() ||
        !project["canvas"]["nodes"].is_array() || !project["canvas"]["edges"].is_array() ||
        !project["canvas"]["groups"].is_array() || !project["canvas"]["viewport"].is_object() ||
        !project["settings"]["captionStyle"].is_object() || !project["settings"]["width"].is_number() ||
        !project["settings"]["height"].is_number() || !project["settings"]["fps"].is_number() {
        return Err(invalid("项目文件缺少当前格式的必要字段"));
    }
    Ok(())
}

fn each_project(app: &App) -> Result<Vec<(PathBuf, Value)>> {
    let mut projects = Vec::new();
    for entry in fs::read_dir(&app.root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Ok(file) = scoped_path(&app.root, &entry.path().join("project.json")) {
            if let Ok(project) = read_json(&file).and_then(|project| {
                validate_project_document(&project)?;
                Ok(project)
            }) {
                projects.push((file, project));
            }
        }
    }
    projects.sort_by(|(_, left), (_, right)| right["updatedAt"].as_u64().cmp(&left["updatedAt"].as_u64()));
    Ok(projects)
}

pub(super) fn project_by_id(app: &App, id: &str) -> Result<(PathBuf, Value)> {
    each_project(app)?.into_iter().find(|(_, item)| item["id"].as_str() == Some(id)).ok_or_else(|| missing("找不到项目"))
}

pub(super) fn active_project(app: &App) -> Result<Option<(PathBuf, Value)>> {
    let id = app.active_project.lock().map_err(|_| invalid("无法读取活动项目"))?.clone();
    let projects = each_project(app)?;
    Ok(match id {
        Some(id) => projects.into_iter().find(|(_, project)| project["id"].as_str() == Some(id.as_str())),
        None => projects.into_iter().next(),
    })
}

pub(super) fn set_active_project(app: &App, id: &str) -> Result<Value> {
    let (_, project) = each_project(app)?.into_iter().find(|(_, item)| item["id"].as_str() == Some(id))
        .ok_or_else(|| missing("找不到指定项目"))?;
    *app.active_project.lock().map_err(|_| invalid("无法切换活动项目"))? = Some(id.to_owned());
    let revision = json!({ "projectId": id, "updatedAt": project["updatedAt"] });
    notify_project_revision(app, revision.clone());
    Ok(revision)
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name].as_str().filter(|text| !text.is_empty()).ok_or_else(|| invalid("缺少必要字段"))
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], |items| items.as_slice())
}

fn paged(source: &[Value], input: &Value) -> Value {
    let offset = input["offset"].as_u64().unwrap_or(0).min(source.len() as u64) as usize;
    let limit = input["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
    let end = (offset + limit).min(source.len());
    json!({ "items": &source[offset..end], "total": source.len(),
        "nextOffset": (end < source.len()).then_some(end) })
}

fn project_meta(project: &Value) -> Value {
    json!({ "id": project["id"], "name": project["name"], "updatedAt": project["updatedAt"],
        "pipelineMode": project["pipelineMode"], "hasScript": project["script"]["content"].as_str().is_some_and(|text| !text.is_empty()),
        "shotCount": array(&project["shots"]).len(), "clipCount": array(&project["timeline"]).len(),
        "finalVideoAssetId": project["finalVideoAssetId"] })
}

pub(super) fn manifest(app: &App, file: &Path) -> Result<Value> {
    let directory = file.parent().ok_or_else(|| missing("找不到项目目录"))?;
    let filename = scoped_path(&app.root, &directory.join("assets/manifest.json"))?;
    if filename.exists() { read_json(&filename) } else { Ok(json!([])) }
}

fn delete_project(app: &App, input: &Value, live_plan: Option<&(String,String)>, live_job: Option<&(String,String)>, allow_active: bool) -> Result<Value> {
    let id = field(input,"projectId")?;
    let mut matching = each_project(app)?.into_iter().filter(|(_, project)| project["id"] == id);
    let (file,project) = matching.next().ok_or_else(|| missing("找不到项目"))?;
    if matching.next().is_some() {
        return Err(invalid("工作区有多个目录使用同一项目 ID，请先确认要保留的目录"));
    }
    let folder = file.parent().ok_or_else(||invalid("项目目录无效"))?;
    if folder.parent() != Some(app.root.as_path()) { return Err(invalid("项目目录超出工作区范围")); }
    let revision = project["updatedAt"].as_u64();
    let active = active_project(app)?.is_some_and(|(_,active)|active["id"] == id);
    let running = [live_plan,live_job].into_iter().flatten().any(|(project,_)|project == id);
    let mut unfinished = Vec::new();
    let state = state_directory(&app.root)?;
    if state.is_dir() {
        for entry in fs::read_dir(&state)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|ext|ext != "json") { continue; }
            let Some(job) = read_json(&entry.path()).ok().filter(|job|job["projectId"] == id && job["requestId"].is_string()) else { continue; };
            let status = job["status"].as_str().unwrap_or_default();
            let unresolved = matches!(status,"submitting"|"waiting"|"paused"|"unknown") ||
                status == "running" && array(&job["steps"]).iter().any(|step|
                    matches!(step["status"].as_str(),Some("submitting"|"waiting")) || step["promptId"].is_string() && step["status"] != "saved");
            if unresolved { unfinished.push(json!({"requestId":job["requestId"],"status":status})); }
        }
    }
    let mut result = json!({"projectId":id,"name":project["name"],"updatedAt":revision,
        "removed":false,"blocked":(active && !allow_active) || running || !unfinished.is_empty(),"active":active,"running":running,
        "unfinishedJobs":unfinished,"shotCount":array(&project["shots"]).len(),"assetCount":array(&manifest(app,&file)?).len()});
    if input["confirmed"] != true || result["blocked"] == true { return Ok(result); }
    validate_project_revision(input, revision.ok_or_else(|| invalid("项目修订号无效"))?, false)?;
    let trash = normalize_root(&state)?.join("removed-projects");
    if fs::symlink_metadata(&trash).is_ok() && normalize_root(&trash)? != trash {
        return Err(invalid("可恢复目录位置无效"));
    }
    fs::create_dir_all(&trash)?;
    if normalize_root(&trash)? != trash || !trash.starts_with(&app.root) {
        return Err(invalid("可恢复目录超出工作区范围"));
    }
    let name = folder.file_name().ok_or_else(||invalid("项目目录名无效"))?;
    let destination = trash.join(format!("{}-{}",video_queue::now(),name.to_string_lossy()));
    if fs::symlink_metadata(&destination).is_ok() { return Err(invalid("移除目标目录已存在，请重试")); }
    fs::rename(folder,&destination).map_err(|error| {
        #[cfg(windows)]
        if matches!(error.raw_os_error(), Some(32 | 33)) {
            return Failure::new(StatusCode::CONFLICT, "项目目录或其中的文件正被其他程序占用，请关闭后再移除".into());
        }
        Failure::from(error)
    })?;
    if active { *app.active_project.lock().map_err(|_| invalid("无法清除活动项目"))? = None; }
    result["removed"] = json!(true);
    result["recoveryPath"] = json!(destination);
    result["originalPath"] = json!(folder);
    Ok(result)
}

fn model_fields(kind: &str) -> Result<(&'static str, &'static str, &'static str)> {
    Ok(match kind {
        "llm" => ("llms", "activeLlmId", "llm"),
        "vision" => ("visions", "activeVisionId", "vision"),
        "image" => ("images", "activeImageId", "image"),
        "video" => ("videos", "activeVideoId", "video"),
        "tts" => ("ttsProfiles", "activeTtsId", "tts"),
        "music" => ("musics", "activeMusicId", "music"),
        _ => return Err(invalid("未知模型类型")),
    })
}

fn list_models(app: &App, kind: &str) -> Result<Value> {
    let (profiles, active, _) = model_fields(kind)?;
    let file = kv_path(app, "settings")?;
    let mut result = json!({"kind":kind,"activeId":null,"profiles":[]});
    if matches!(kind,"image" | "video") {
        result["workflowDiscovery"] = json!({
            "scope":"saved_profiles",
            "discoverTool":"studio_comfy_discover",
            "listTool":"studio_comfy_workflows",
            "importTool":"studio_comfy_import",
            "message":"这里只列 Dramlo 已保存的接入。直接调用 studio_comfy_workflows 浏览唯一在线实例的工作流。多个实例或离线安装请通过 studio_comfy_discover 选择目标。"
        });
    }
    if !file.exists() { return Ok(result); }
    let record = read_json(&file)?;
    if record["key"].as_str() != Some("settings") {
        return Err(invalid("设置键无效"));
    }
    let settings: Value = serde_json::from_str(field(&record, "value")?)
        .map_err(|_| invalid("设置格式无效"))?;
    let profiles = array(&settings[profiles]).iter().map(|profile| json!({
        "id": profile["id"], "name": profile["name"], "provider": profile["provider"], "providerName":profile["providerName"],
        "model": if kind == "video" { &profile["workflowId"] } else { &profile["model"] },
        "adapter": profile["adapter"]["type"],
        "serverExecutionSupported": match kind {
            "image" => image_queue::endpoint_check(profile).is_ok(),
            "video" => video_queue::endpoint(profile).is_ok(),
            _ => false,
        },
        "hasCredential": profile["apiKey"].as_str().is_some_and(|key| !key.is_empty())
    })).collect::<Vec<_>>();
    result["profiles"] = json!(profiles);
    result["activeId"] = settings[active].clone();
    Ok(result)
}

pub(super) fn default_model(app: &App, kind: &str) -> Result<Value> {
    let models = list_models(app, kind)?;
    let Some(active) = models["activeId"].as_str() else { return Ok(Value::Null); };
    Ok(array(&models["profiles"]).iter().find(|profile| profile["id"] == active).cloned().unwrap_or_default())
}

fn describe_model(app: &App, input: &Value) -> Result<Value> {
    let kind = field(input, "kind")?;
    let id = field(input, "profileId")?;
    let (group, active_key, _) = model_fields(kind)?;
    let (_, settings) = managed_comfy::settings(app)?;
    let profile = array(&settings[group]).iter().find(|profile| profile["id"] == id)
        .ok_or_else(|| missing("找不到指定配置，请用 studio_list_models 核对类型和配置 ID"))?;
    describe_saved_model(kind, id, profile, settings[active_key] == id)
}

fn describe_saved_model(kind: &str, id: &str, profile: &Value, active: bool) -> Result<Value> {
    let adapter = &profile["adapter"];
    let workflow_origin = &adapter["workflowOrigin"];
    let workflow_origin = if workflow_origin.is_object() {
        json!({"instanceId":workflow_origin["instanceId"],"source":workflow_origin["source"],
            "root":workflow_origin["root"],"relativePath":workflow_origin["relativePath"]})
    } else { Value::Null };
    let address = profile["baseUrl"].as_str().and_then(|raw| model_address(raw).ok())
        .and_then(|raw| raw.parse::<axum::http::Uri>().ok());
    let origin = address.as_ref().and_then(|uri| Some(format!("{}://{}", uri.scheme_str()?, uri.authority()?)));
    let mapping: Vec<_> = adapter["inputMap"]["bindings"].as_array().into_iter().flatten().map(|binding| {
        json!({"role":binding["role"],"nodeId":binding["nodeId"],"field":binding["field"],
            "mediaMode":binding["mediaMode"],"durationUnit":binding["durationUnit"]})
    }).collect();
    let endpoint_issue = match kind {
        "image" => image_queue::endpoint_check(profile).err().map(|error| error.1),
        "video" => video_queue::endpoint(profile).err().map(|error| error.1),
        _ => Some("原生 Web 队列支持图像和视频。此类型由外部 Agent 或现有界面使用。".into()),
    };
    let input_issue = if kind == "image" && adapter["type"] == "openai-images" {
        openai_images::ready(profile).err().map(|error| error.1)
    } else { None };
    let workflow = if adapter["type"] == "comfyui-prompt" {
        match kind {
            "image" => {
                let issue = image_queue::mapping_issue(profile);
                json!({"ready":issue.is_none(),"issue":issue,"missingMapping":image_queue::missing_mapping(profile)})
            }
            "video" => video_queue::describe_workflow(profile)
                .unwrap_or_else(|error| json!({"ready":false,"issue":error.1})),
            _ => Value::Null,
        }
    } else { Value::Null };
    let references = match kind {
        "image" if endpoint_issue.is_none() => image_queue::describe_profile_inputs(profile).ok(),
        "video" if workflow["referenceSlots"].is_array() => Some(json!({
            "referenceSlots":workflow["referenceSlots"],"referenceMode":workflow["referenceMode"],
            "minReferences":workflow["minReferences"],"maxReferences":workflow["maxReferences"]
        })),
        _ => None,
    };
    let refs = references.unwrap_or_default();
    Ok(json!({"kind":kind,"profileId":id,"name":profile["name"],"active":active,
        "provider":profile["provider"],"providerName":profile["providerName"],"adapterType":adapter["type"],
        "model":if kind == "video" { &profile["workflowId"] } else { &profile["model"] },
        "endpointOrigin":origin,"hasEndpointPath":address.as_ref().is_some_and(|uri| uri.path() != "/"),
        "hasCredential":profile["apiKey"].as_str().is_some_and(|key| !key.is_empty()) ||
            adapter["headers"].as_object().is_some_and(|headers| !headers.is_empty()),
        "workflowOrigin":workflow_origin,"inputMap":{"bindings":mapping},
        "referenceConfig":adapter["referenceConfig"],"outputConfig":adapter["outputConfig"],
        "workflowRevision":workflow_revision(profile),
        "serverExecutionSupported":endpoint_issue.is_none(),"endpointIssue":endpoint_issue,
        "ready":endpoint_issue.is_none() && input_issue.is_none() && (workflow["ready"] == true ||
            (kind == "image" && adapter["type"] == "openai-images")),
        "inputIssue":input_issue,"mappingChecked":workflow["missingMapping"].is_array(),
        "missingMapping":workflow["missingMapping"],
        "referenceSlots":refs["referenceSlots"],"referenceMode":refs["referenceMode"],
        "minReferences":refs["minReferences"],"maxReferences":refs["maxReferences"],
        "workflow":workflow,"connectionChecked":false}))
}

fn workflow_revision(profile: &Value) -> Option<String> {
    let adapter = &profile["adapter"];
    adapter["workflowTemplate"].as_str()?;
    Some(revision_string(&json!([adapter["workflowTemplate"],adapter["inputMap"]]).to_string()))
}

fn broken_bindings(graph: &Value, mapping: &Value) -> Vec<Value> {
    array(&mapping["bindings"]).iter().filter_map(|binding| {
        let node_id = binding["nodeId"].as_str().unwrap_or_default();
        let key = binding["field"].as_str().unwrap_or_default();
        let reason = if graph.get(node_id).is_none() { "missing_node" }
            else if graph[node_id]["inputs"].get(key).is_none() { "missing_field" }
            else if graph[node_id]["inputs"][key].is_array() || graph[node_id]["inputs"][key].is_object() { "linked_input" }
            else { return None; };
        Some(json!({"role":binding["role"],"nodeId":node_id,"field":key,"reason":reason}))
    }).collect()
}

fn patch_workflow(profile: &mut Value, input: &Value) -> Result<Value> {
    let revision = workflow_revision(profile).ok_or_else(|| invalid("此配置没有 API 工作流"))?;
    if input["expectedWorkflowRevision"] != revision {
        return Err(Failure::new(StatusCode::CONFLICT,"工作流或映射已修改，请重新读取配置".into()));
    }
    let mut template: Value = serde_json::from_str(field(&profile["adapter"],"workflowTemplate")?)
        .map_err(|_| invalid("工作流 JSON 无效"))?;
    let nodes = if template.get("prompt").is_some() { &mut template["prompt"] } else { &mut template };
    let patches = input["patches"].as_array().filter(|items| (1..=64).contains(&items.len()))
        .ok_or_else(|| invalid("请提供 1 至 64 项工作流修改"))?;
    let mut used = std::collections::HashSet::new();
    let mut result = Vec::new();
    for patch in patches {
        let node = field(patch,"nodeId")?;
        let key = field(patch,"field")?;
        if !used.insert((node,key)) { return Err(invalid("同一个工作流输入只能改一次")); }
        let target = nodes.get_mut(node).and_then(|node|node.get_mut("inputs")).and_then(|inputs|inputs.get_mut(key))
            .ok_or_else(|| invalid(&format!("找不到工作流输入 {node}.{key}")))?;
        let new = &patch["value"];
        if !(target.is_string() && new.is_string() || target.is_number() && new.is_number() || target.is_boolean() && new.is_boolean()) {
            return Err(invalid(&format!("输入 {node}.{key} 只支持同类型的文本、数字或布尔值修改")));
        }
        *target = new.clone();
        let role = array(&profile["adapter"]["inputMap"]["bindings"]).iter().find(|item|item["nodeId"] == node && item["field"] == key)
            .map(|item|item["role"].clone());
        result.push(json!({"nodeId":node,"field":key,"mappedRole":role}));
    }
    let broken = broken_bindings(nodes,&profile["adapter"]["inputMap"]);
    profile["adapter"]["workflowTemplate"] = json!(template.to_string());
    Ok(json!({"workflowRevision":workflow_revision(profile),"patched":result,"brokenBindings":broken}))
}

fn write_credential(profile: &mut Value, input: &Value) -> Result<()> {
    let expected = model_address(field(input,"expectedBaseUrl")?)?;
    if profile["baseUrl"] != expected {
        return Err(Failure::new(StatusCode::CONFLICT,"模型地址已变化，请核对配置后再写入密钥".into()));
    }
    let secret = match input.get("apiKey") {
        Some(Value::Null) => "",
        Some(value) => value.as_str().filter(|text| !text.is_empty() && text.len() <= 8192
            && !text.chars().any(char::is_whitespace) && !text.chars().any(char::is_control))
            .ok_or_else(||invalid("密钥格式无效"))?,
        None => return Err(invalid("请提供密钥，清除时传 null")),
    };
    if !secret.is_empty() && !expected.starts_with("https://") &&
        !expected.parse::<axum::http::Uri>().ok().and_then(|uri|uri.host().map(str::to_owned))
            .is_some_and(|host|matches!(host.as_str(),"localhost"|"127.0.0.1"|"[::1]")) {
        return Err(invalid("在线模型写入密钥需要 HTTPS"));
    }
    profile["apiKey"] = json!(secret);
    Ok(())
}

fn summarize_shot(shot: &Value, start: f64, end: f64) -> Value {
    json!({ "id":shot["id"],"index":shot["index"],"title":shot["title"],"duration":shot["duration"],
        "sceneId":shot["sceneId"],"chapterId":shot["chapterId"],
        "plannedStartSec":start,"plannedEndSec":end })
}

fn update_model_adapter(profile: &mut Value, input: &Value) -> Result<()> {
    if input["adapterType"].is_string() || input["workflowTemplate"].is_string() || input.get("inputMap").is_some() {
        if !profile["adapter"].is_object() { profile["adapter"] = json!({}); }
        if let Some(adapter) = input["adapterType"].as_str() {
            profile["adapter"]["type"] = json!(model_adapter(adapter)?);
        }
        if let Some(template) = input["workflowTemplate"].as_str() {
            let raw: Value = serde_json::from_str(template).map_err(|_| invalid("工作流 JSON 无效"))?;
            let nodes = raw.get("prompt").unwrap_or(&raw);
            if !nodes.as_object().is_some_and(|items| !items.is_empty() && items.values().all(|node|
                node["class_type"].is_string() && node["inputs"].is_object()
            )) { return Err(invalid("工作流须为 API 格式")); }
            if input.get("inputMap").is_none() && profile["adapter"]["workflowTemplate"] != template {
                profile["adapter"].as_object_mut().unwrap().remove("inputMap");
            }
            profile["adapter"]["workflowTemplate"] = json!(template);
        }
        if let Some(map) = input.get("inputMap") {
            if !map.is_object() || !profile["adapter"]["workflowTemplate"].is_string() {
                return Err(invalid("请先保存 API 工作流，再更新映射对象"));
            }
            profile["adapter"]["inputMap"] = map.clone();
        }
    }
    Ok(())
}

fn model_adapter(adapter: &str) -> Result<&str> {
    if ["openai-compatible", "openai-images", "http-custom", "comfyui-workflow", "zealman-workflow", "comfyui-prompt"].contains(&adapter) {
        Ok(adapter)
    } else { Err(invalid("适配器类型无效")) }
}

fn remove_model(app: &App, input: &Value) -> Result<Value> {
    let kind = field(input, "kind")?;
    let id = field(input, "profileId")?;
    let (group, active, mirror) = model_fields(kind)?;
    let (path, mut settings) = managed_comfy::settings(app)?;
    let profile = array(&settings[group]).iter().find(|profile| profile["id"] == id)
        .ok_or_else(|| missing("找不到指定配置"))?;
    let is_default = settings[active] == id || settings[mirror]["id"] == id;
    let mut projects = Vec::new();
    let mut jobs = Vec::new();
    let extensions = if kind == "video" { canvas_extensions::using_video_profile(app,id)? } else { vec![] };
    if kind == "video" {
        for (_, project) in each_project(app)? {
            let shots: Vec<_> = array(&project["shots"]).iter().filter(|shot| shot["videoProfileId"] == id)
                .map(|shot| &shot["id"]).collect();
            let nodes: Vec<_> = array(&project["canvas"]["nodes"]).iter().filter(|node| node["data"]["videoProfileId"] == id)
                .map(|node| &node["id"]).collect();
            if !shots.is_empty() || !nodes.is_empty() {
                projects.push(json!({"projectId":project["id"],"shotIds":shots,"nodeIds":nodes}));
            }
        }
    }
    if matches!(kind, "video" | "image") {
        for entry in fs::read_dir(state_directory(&app.root)?)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|ext| ext != "json") { continue; }
            let Some(record) = fs::read(entry.path()).ok().and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else { continue; };
            let status = record["status"].as_str().unwrap_or_default();
            let uses_model = if kind == "video" {
                array(&record["steps"]).iter().any(|step| step["profileId"] == id && step["status"] != "saved")
            } else { record["targetType"].is_string() && record["profileId"] == id };
            if uses_model && matches!(status,"prepared"|"starting"|"ready"|"running"|"submitting"|"waiting"|"paused"|"unknown") {
                jobs.push(json!({"projectId":record["projectId"],"requestId":record["requestId"],"status":status,"blocksRemoval":status != "prepared"}));
            }
        }
    }
    let blocked = is_default || !projects.is_empty() || !extensions.is_empty() || jobs.iter().any(|job| job["blocksRemoval"] == true);
    let diagnosis = describe_saved_model(kind,id,profile,is_default)?;
    let mut result = json!({"kind":kind,"profileId":id,"removed":false,"blocked":blocked,"active":is_default,
        "projects":projects,"extensions":extensions,"jobs":jobs,
        "ready":diagnosis["ready"],"missingMapping":diagnosis["missingMapping"],"mappingChecked":diagnosis["mappingChecked"],
        "hasCredential":diagnosis["hasCredential"],
        "issue":diagnosis["endpointIssue"].as_str().or_else(|| diagnosis["inputIssue"].as_str())
            .or_else(|| diagnosis["workflow"]["issue"].as_str())});
    if input["confirmed"] == true && !blocked {
        settings[group].as_array_mut().ok_or_else(|| invalid("模型配置格式无效"))?.retain(|profile| profile["id"] != id);
        atomic_save(&path,json!({"key":"settings","value":settings.to_string()}).to_string().as_bytes())?;
        result["removed"] = json!(true);
    }
    Ok(result)
}

fn change_model(app: &App, name: &str, input: &Value) -> Result<Value> {
    let kind = field(input, "kind")?;
    let (profiles_key, active_key, mirror_key) = model_fields(kind)?;
    let location = kv_path(app, "settings")?;
    let mut settings: Value = if location.exists() {
        let record = read_json(&location)?;
        if record["key"] != "settings" { return Err(invalid("设置键无效")); }
        serde_json::from_str(field(&record, "value")?).map_err(|_| invalid("设置格式无效"))?
    } else { json!({}) };
    if !settings.is_object() { return Err(invalid("设置格式无效")); }
    if settings[profiles_key].is_null() { settings[profiles_key] = json!([]); }
    let active = settings[active_key].as_str().map(str::to_owned);
    let providers = settings[profiles_key].as_array_mut().ok_or_else(|| invalid("模型配置格式无效"))?;
    let model_key = if kind == "video" { "workflowId" } else { "model" };
    let mut changes = Value::Null;
    let selected = if name == "studio_add_model" {
        let address = model_address(field(input, "baseUrl")?)?;
        let label = field(input, "name")?;
        let model = field(input, "model")?;
        let provider = input["provider"].as_str().unwrap_or("custom");
        if !["custom", "openai", "deepseek", "dashscope", "apimart"].contains(&provider) {
            return Err(invalid("供应商类型无效"));
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let id = format!("{kind}-web-{now}");
        let mut profile = json!({ "id": id, "name": label, "provider": provider, "baseUrl": address, "apiKey": "" });
        update_model_adapter(&mut profile,input)?;
        comfy_config::update(&mut profile,input,kind)?;
        if let Some(label) = input["providerName"].as_str().map(str::trim).filter(|name| !name.is_empty() && name.chars().count() <= 120) {
            profile["providerName"] = json!(label);
        }
        profile[model_key] = json!(model);
        providers.push(profile.clone());
        profile
    } else {
        let id = field(input, "profileId")?;
        let profile = providers.iter_mut().find(|profile| profile["id"].as_str() == Some(id))
            .ok_or_else(|| missing("找不到指定配置"))?;
        if name == "studio_set_credential" { write_credential(profile,input)?; }
        if name == "studio_patch_workflow" {
            if !matches!(kind,"image"|"video") { return Err(invalid("工作流局部修改只支持图像和视频配置")); }
            changes = patch_workflow(profile,input)?;
        }
        if name == "studio_model_update" {
            let has_credentials = profile["apiKey"].as_str().is_some_and(|key| !key.is_empty())
                || profile["adapter"]["headers"].is_object()
                || matches!(profile["provider"].as_str(),Some("autodl"|"zealman"));
            for key in ["name","providerName"] {
                if let Some(value) = input.get(key) {
                    if key == "providerName" && value.is_null() {
                        profile.as_object_mut().ok_or_else(|| invalid("模型配置格式无效"))?.remove(key);
                    } else {
                        let value = value.as_str().map(str::trim).filter(|label| !label.is_empty() && label.chars().count() <= 120)
                            .ok_or_else(|| invalid("配置名称或供应商显示名无效"))?;
                        profile[key] = json!(value);
                    }
                }
            }
            if let Some(value) = input.get("provider") {
                let provider = value.as_str().filter(|provider| ["custom","openai","deepseek","dashscope","apimart"].contains(provider))
                    .ok_or_else(|| invalid("供应商类型无效"))?;
                if has_credentials && profile["provider"] != provider {
                    return Err(invalid("此配置关联凭据，请在设置页修改供应商类型"));
                }
                profile["provider"] = json!(provider);
            }
            if let Some(model) = input["model"].as_str() { profile[model_key] = json!(model); }
            if let Some(address) = input["baseUrl"].as_str() {
                let address = model_address(address)?;
                if has_credentials && profile["baseUrl"] != address {
                    return Err(invalid("此配置关联凭据。请先新建目标地址的配置，在设置页填写凭据并验证，再切换默认项和镜头引用，最后清理旧配置。"));
                }
                profile["baseUrl"] = json!(address);
            }
            if let Some(voice) = input["voice"].as_str() { profile["voice"] = json!(voice); }
            let original_map = profile["adapter"]["inputMap"].clone();
            update_model_adapter(profile,input)?;
            comfy_config::update(profile,input,kind)?;
            if input.get("workflowTemplate").is_some() || input.get("inputMap").is_some() {
                if let Ok(graph) = video_queue::workflow_template(profile) {
                    let mapping = profile["adapter"].get("inputMap").unwrap_or(&original_map);
                    changes = json!({"brokenBindings":broken_bindings(&graph,mapping)});
                }
            }
        }
        profile.clone()
    };
    let activate = match name {
        "studio_add_model" => input["select"].as_bool().unwrap_or(true),
        "studio_select_model" => true,
        _ => active.as_deref() == selected["id"].as_str(),
    };
    if activate {
        settings[active_key] = selected["id"].clone();
        settings[mirror_key] = selected.clone();
    }
    atomic_save(&location, json!({ "key": "settings", "value": settings.to_string() }).to_string().as_bytes())?;
    if name == "studio_set_credential" {
        return Ok(json!({"kind":kind,"profileId":selected["id"],"hasCredential":selected["apiKey"] != ""}));
    }
    if name == "studio_patch_workflow" {
        changes["profileId"] = selected["id"].clone();
        return Ok(changes);
    }
    let mut result = list_models(app, kind)?;
    result["profileId"] = selected["id"].clone();
    if let Some(broken) = changes.get("brokenBindings") { result["brokenBindings"] = broken.clone(); }
    Ok(result)
}

const SKILLS: &[&str] = &[
    include_str!("../../src/skills/codex-short-drama/ai-short-film-screenwriter.md"),
    include_str!("../../src/skills/codex-short-drama/SKILL.md"),
    include_str!("../../src/skills/codex-short-drama/storyboard-director.md"),
    include_str!("../../src/skills/codex-short-drama/character-consistency.md"),
    include_str!("../../src/skills/codex-short-drama/storyboard-panel.md"),
    include_str!("../../src/skills/codex-short-drama/shot-image.md"),
    include_str!("../../src/skills/codex-short-drama/shot-video.md"),
    include_str!("../../src/skills/codex-short-drama/h3-prompt-writing.md"),
    include_str!("../../src/skills/codex-short-drama/qa-verify.md"),
];

const DRAMA_FLOW_BODY: &str = include_str!("../../src/skills/drama-flow-explore/SKILL.md");
const DRAMA_FLOW_REFERENCES: &[(&str, &str)] = &[
    ("references/assets.md", include_str!("../../src/skills/drama-flow-explore/references/assets.md")),
    ("references/continuity.md", include_str!("../../src/skills/drama-flow-explore/references/continuity.md")),
    ("references/director.md", include_str!("../../src/skills/drama-flow-explore/references/director.md")),
    ("references/edit.md", include_str!("../../src/skills/drama-flow-explore/references/edit.md")),
    ("references/example-handover.md", include_str!("../../src/skills/drama-flow-explore/references/example-handover.md")),
    ("references/example-offscreen-promise.md", include_str!("../../src/skills/drama-flow-explore/references/example-offscreen-promise.md")),
    ("references/final-prompt.md", include_str!("../../src/skills/drama-flow-explore/references/final-prompt.md")),
    ("references/keyframe.md", include_str!("../../src/skills/drama-flow-explore/references/keyframe.md")),
    ("references/panel.md", include_str!("../../src/skills/drama-flow-explore/references/panel.md")),
    ("references/production.md", include_str!("../../src/skills/drama-flow-explore/references/production.md")),
    ("references/review.md", include_str!("../../src/skills/drama-flow-explore/references/review.md")),
    ("references/scene.md", include_str!("../../src/skills/drama-flow-explore/references/scene.md")),
    ("references/screenplay.md", include_str!("../../src/skills/drama-flow-explore/references/screenplay.md")),
    ("references/sound.md", include_str!("../../src/skills/drama-flow-explore/references/sound.md")),
    ("references/story.md", include_str!("../../src/skills/drama-flow-explore/references/story.md")),
    ("references/storyboard.md", include_str!("../../src/skills/drama-flow-explore/references/storyboard.md")),
    ("references/video.md", include_str!("../../src/skills/drama-flow-explore/references/video.md")),
    ("references/source-assets-boards.md", include_str!("../../src/skills/drama-flow-explore/references/source-assets-boards.md")),
    ("references/source-assets-extraction.md", include_str!("../../src/skills/drama-flow-explore/references/source-assets-extraction.md")),
    ("references/source-assets-recipes.md", include_str!("../../src/skills/drama-flow-explore/references/source-assets-recipes.md")),
    ("references/source-continuity-cuts.md", include_str!("../../src/skills/drama-flow-explore/references/source-continuity-cuts.md")),
    ("references/source-continuity-state.md", include_str!("../../src/skills/drama-flow-explore/references/source-continuity-state.md")),
    ("references/source-director-acting.md", include_str!("../../src/skills/drama-flow-explore/references/source-director-acting.md")),
    ("references/source-director-performance.md", include_str!("../../src/skills/drama-flow-explore/references/source-director-performance.md")),
    ("references/source-director-staging.md", include_str!("../../src/skills/drama-flow-explore/references/source-director-staging.md")),
    ("references/source-edit-delivery.md", include_str!("../../src/skills/drama-flow-explore/references/source-edit-delivery.md")),
    ("references/source-image-keyframes.md", include_str!("../../src/skills/drama-flow-explore/references/source-image-keyframes.md")),
    ("references/source-index.md", include_str!("../../src/skills/drama-flow-explore/references/source-index.md")),
    ("references/source-panel-sequence.md", include_str!("../../src/skills/drama-flow-explore/references/source-panel-sequence.md")),
    ("references/source-production-controls.md", include_str!("../../src/skills/drama-flow-explore/references/source-production-controls.md")),
    ("references/source-production-route.md", include_str!("../../src/skills/drama-flow-explore/references/source-production-route.md")),
    ("references/source-prompt-compiler.md", include_str!("../../src/skills/drama-flow-explore/references/source-prompt-compiler.md")),
    ("references/source-review-media.md", include_str!("../../src/skills/drama-flow-explore/references/source-review-media.md")),
    ("references/source-review-methods.md", include_str!("../../src/skills/drama-flow-explore/references/source-review-methods.md")),
    ("references/source-scene-layout.md", include_str!("../../src/skills/drama-flow-explore/references/source-scene-layout.md")),
    ("references/source-scene-specialist.md", include_str!("../../src/skills/drama-flow-explore/references/source-scene-specialist.md")),
    ("references/source-screenplay-adaptation.md", include_str!("../../src/skills/drama-flow-explore/references/source-screenplay-adaptation.md")),
    ("references/source-sound-design.md", include_str!("../../src/skills/drama-flow-explore/references/source-sound-design.md")),
    ("references/source-story-adaptation.md", include_str!("../../src/skills/drama-flow-explore/references/source-story-adaptation.md")),
    ("references/source-storyboard-camera.md", include_str!("../../src/skills/drama-flow-explore/references/source-storyboard-camera.md")),
    ("references/source-storyboard-handoff.md", include_str!("../../src/skills/drama-flow-explore/references/source-storyboard-handoff.md")),
    ("references/source-storyboard-timing.md", include_str!("../../src/skills/drama-flow-explore/references/source-storyboard-timing.md")),
    ("references/source-video-motion.md", include_str!("../../src/skills/drama-flow-explore/references/source-video-motion.md")),
];

fn drama_flow_skill(selected: Option<&str>, reference: Option<&str>) -> Result<Value> {
    let raw = DRAMA_FLOW_BODY.replace("\r\n", "\n");
    let (metadata, body) = raw.strip_prefix("---\n").and_then(|text| text.split_once("\n---\n"))
        .ok_or_else(|| invalid("内置 Skill 元数据格式无效"))?;
    let description = metadata.lines().find_map(|line| line.strip_prefix("description:"))
        .unwrap_or("").trim();
    let mut revision = content_revision(&raw);
    for (path, content) in DRAMA_FLOW_REFERENCES {
        revision = revision_hash(revision, path.as_bytes());
        revision = revision_hash(revision, content.as_bytes());
    }
    let references: Vec<&str> = DRAMA_FLOW_REFERENCES.iter().map(|(path, _)| *path).collect();
    let mut item = json!({ "id":"drama-flow-explore", "name":"通用 AI 短剧创作",
        "description":description, "source":"studio-built-in", "stage":"planning",
        "trigger":"从小说改编到逐集成片，按创作任务选读",
        "revision":format!("{revision:016x}"), "references":references });
    if selected.is_some() {
        item["body"] = json!(match reference {
            Some(path) => DRAMA_FLOW_REFERENCES.iter().find(|(name, _)| *name == path)
                .map(|(_, content)| *content).ok_or_else(|| missing("找不到指定 Skill 参考资料"))?,
            None => body.trim(),
        });
        item["referencePath"] = json!(reference);
    }
    Ok(item)
}

fn local_skill_entries(selected: Option<&str>, reference: Option<&str>) -> Result<Vec<Value>> {
    let executable = env::current_exe()?;
    let root = executable.parent().ok_or_else(|| invalid("找不到发行包目录"))?.join("creative-skills");
    if !root.is_dir() { return Ok(vec![]); }
    let root = normalize_root(&root)?;
    let mut items = Vec::new();
    for directory in fs::read_dir(&root)?.flatten().take(100) {
        let id = directory.file_name().to_string_lossy().into_owned();
        if !directory.file_type().is_ok_and(|kind| kind.is_dir())
            || !id.starts_with("laotang-") || !id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || selected.is_some_and(|selected| selected != id) { continue; }
        let read = |relative: &Path| -> Result<String> {
            let file = scoped_path(&root, &Path::new(&id).join(relative))?;
            if fs::metadata(&file)?.len() > 256 * 1024 { return Err(invalid("Skill 文档过大")); }
            Ok(fs::read_to_string(file)?)
        };
        let raw = read(Path::new("SKILL.md"))?.replace("\r\n", "\n");
        let (metadata, body) = raw.trim_start_matches('\u{feff}').strip_prefix("---\n")
            .and_then(|raw| raw.split_once("\n---\n")).ok_or_else(|| invalid("本机 Skill 元数据格式无效"))?;
        let description = metadata.lines().find_map(|line| line.strip_prefix("description:"))
            .unwrap_or("").trim().trim_matches('"');
        let title = read(Path::new("title.txt"))?;
        let mut references = Vec::new();
        if let Ok(files) = fs::read_dir(directory.path().join("references")) {
            for file in files.flatten().take(100) {
                let name = file.file_name().to_string_lossy().into_owned();
                if file.file_type().is_ok_and(|kind| kind.is_file()) && name.ends_with(".md")
                    && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')) {
                    references.push(format!("references/{name}"));
                }
            }
        }
        references.sort();
        let revision = references.iter().try_fold(content_revision(&raw), |mut revision, path| -> Result<u64> {
            revision = revision_hash(revision, path.as_bytes());
            Ok(revision_hash(revision, read(Path::new(path))?.as_bytes()))
        })?;
        let mut item = json!({ "id": id, "name": title.trim(), "description": description, "revision": format!("{revision:016x}"),
            "source": "studio-local-bundle", "references": references });
        if selected.is_some() {
            item["body"] = if let Some(reference) = reference {
                if !references.iter().any(|path| path == reference) { return Err(missing("找不到指定 Skill 参考资料")); }
                json!(read(Path::new(reference))?)
            } else {
                json!(format!("本机只提供创作正文和列出的 Markdown 参考。路径请用 studio_read_skill 的 referencePath 读取。原文中脚本和生成步骤按当前用户授权与可用工具处理。\n\n{}", body.trim()))
            };
            item["referencePath"] = json!(reference);
        }
        items.push(item);
    }
    items.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    Ok(items)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillQuery { skill_id: Option<String>, reference_path: Option<String> }

pub(super) async fn local_skills_browser(Query(query): Query<SkillQuery>) -> Result<Json<Value>> {
    if query.skill_id.as_deref() == Some("drama-flow-explore") {
        return Ok(Json(drama_flow_skill(query.skill_id.as_deref(), query.reference_path.as_deref())?));
    }
    let mut items = local_skill_entries(query.skill_id.as_deref(), query.reference_path.as_deref())?;
    if query.skill_id.is_none() { items.push(drama_flow_skill(None, None)?); }
    if query.skill_id.is_some() {
        return items.pop().map(Json).ok_or_else(|| missing("找不到本机 Skill"));
    }
    Ok(Json(json!({ "items": items })))
}

fn skill_route(item: &mut Value) {
    let (stage, trigger) = match item["id"].as_str().unwrap_or_default() {
        "drama-flow-explore" => ("planning","从小说改编到逐集成片，按创作任务选读"),
        "pipeline-batch" => ("planning","安排整场、整段或整集的依赖、样片和验收"),
        "ai-short-film-screenwriter" => ("writing","创作或修订短剧故事与剧本"),
        "storyboard-director" => ("storyboard","从已确认剧本拆文字分镜，定切点和机位"),
        "storyboard-panel" => ("storyboard","现有分镜需要构图与拍点看板时选用"),
        "laotang-storycraft-zh" => ("writing","构思剧情、组织故事或润色剧本"),
        "laotang-cinematic-storyboard-s10-free" => ("storyboard","按电影镜头语言拆分剧本"),
        "laotang-asset-extractor" | "laotang-character-board" | "laotang-scene-reference-zh" =>
            ("assets","从剧本提取或设计人物、场景和道具"),
        "studio-imported-director" => ("planning","读取用户导入的导演手册"),
        "character-consistency" => ("assets","建立角色或核对跨镜造型"),
        "shot-image" => ("image","制作镜头静帧或参考图"),
        "shot-video" => ("video","准备镜头动作、表演与视频输入"),
        "h3-prompt-writing" => ("video","按实际输入编写 Dramlo H3 分镜出片正文"),
        "qa-verify" => ("verification","核对生成结果与交付条件"),
        _ => ("reference","有明确任务需求时按描述选择"),
    };
    item["stage"] = json!(stage);
    item["trigger"] = json!(trigger);
}

fn studio_skills(app: &App, selected: Option<&str>, reference: Option<&str>) -> Result<Value> {
    if selected == Some("drama-flow-explore") {
        return drama_flow_skill(selected, reference);
    }
    let mut entries = local_skill_entries(selected, reference)?;
    if selected.is_none() { entries.push(drama_flow_skill(None, None)?); }
    if selected.is_some() {
        if let Some(item) = entries.pop() { return Ok(item); }
        if reference.is_some() { return Err(missing("找不到指定 Skill 参考资料")); }
    }
    for raw in SKILLS {
        let normalized = raw.replace("\r\n", "\n");
        let Some((metadata, body)) = normalized.trim_start_matches('\u{feff}').strip_prefix("---\n")
            .and_then(|text| text.split_once("\n---\n")) else { continue; };
        let pick = |key: &str| metadata.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (key == name).then(|| value.trim())
        }).unwrap_or("");
        let id = pick("name");
        let mut item = json!({ "id": id, "name": id, "description": pick("description"), "source": "studio-built-in", "revision": revision_string(&normalized) });
        if selected == Some(id) {
            item["body"] = json!(body.trim());
            skill_route(&mut item);
            return Ok(item);
        }
        entries.push(item);
    }
    let path = kv_path(app, "settings")?;
    if path.exists() {
        let record = read_json(&path)?;
        let settings: Value = serde_json::from_str(field(&record, "value")?).map_err(|_| invalid("设置格式无效"))?;
        let director = &settings["directorSkill"];
        if let Some(body) = director["body"].as_str() {
            let mut item = json!({ "id": "studio-imported-director", "name": director["name"],
                "description": "用户导入的导演手册", "source": "studio-imported", "revision": revision_string(body) });
            if selected == Some("studio-imported-director") {
                item["body"] = json!(body);
                return Ok(item);
            }
            entries.push(item);
        }
    }
    if selected.is_some() { return Err(missing("找不到指定的 Dramlo Skill")); }
    for item in &mut entries { skill_route(item); }
    Ok(json!({ "items": entries, "usage": SKILL_USAGE }))
}

pub(super) fn model_address(raw: &str) -> Result<&str> {
    let uri = raw.parse::<axum::http::Uri>().map_err(|_| invalid("模型地址无效"))?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.authority().is_none_or(|value| value.as_str().contains('@'))
        || uri.query().is_some() || raw.contains('#') {
        return Err(invalid("请输入不含凭据的 HTTP(S) 地址"));
    }
    Ok(raw)
}

pub(super) const SKILL_USAGE: &str = "Dramlo 提供影视短剧制作 Skill 库。创作时按阶段用 studio_list_skills 找匹配项，再用 studio_read_skill 直接读取正文和所需参考。工具能力和参数见 tools/list，操作顺序与确认边界见 studio_get_guide。无匹配项时照常创作。禁止把 Dramlo Skill 安装、复制或注册到外部 Agent 的技能目录、配置或工作区。";

const GUIDE: &str = include_str!("../../docs/studio-operation-manual.md");

fn revision_hash(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes { hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3); }
    hash
}

fn content_revision(value: &str) -> u64 {
    revision_hash(0xcbf29ce484222325, value.as_bytes())
}

fn revision_string(value: &str) -> String {
    format!("{:016x}", content_revision(value))
}

fn guide(input: &Value) -> Result<Value> {
    let text = GUIDE.replace("\r\n", "\n");
    let sections = [
        ("startup", "1. 启动与数据"),
        ("models", "2. 模型、供应商和工作流"),
        ("setup", "3. 接入外部 Agent"),
        ("writing", "4. 剧本、分镜与画布"),
        ("production", "5. 生成、暂停和找回素材"),
        ("comfyui", "ComfyUI 工作流发现"),
        ("runtime", "本机 Web 工作区"),
    ];
    let revision = revision_string(&text);
    let Some(section) = input["section"].as_str() else {
        return Ok(json!({ "revision": revision, "sections": sections.map(|(id, title)| json!({ "id": id, "title": title })),
            "summary": "先核对当前项目和修订号。需要创作规则时读取 Dramlo Skill。确认前只做准备与预检。" }));
    };
    let (_, title) = sections.iter().find(|(id, _)| *id == section).ok_or_else(|| missing("找不到指定操作指南"))?;
    let heading = format!("## {title}\n");
    let body = text.split_once(&heading).map(|(_, rest)| rest.split("\n## ").next().unwrap_or("").trim())
        .ok_or_else(|| missing("找不到指定操作指南正文"))?;
    Ok(json!({ "section": section, "title": title, "revision": revision, "body": body }))
}

pub(super) fn native_method(name: &str) -> bool {
    METHODS.contains(&name) || video_queue::METHODS.contains(&name) || final_export::METHODS.contains(&name) ||
        project_authoring::METHODS.contains(&name) || canvas_write::METHODS.contains(&name) || capture_plan::METHODS.contains(&name) || managed_comfy::METHODS.contains(&name) || canvas_extensions::METHODS.contains(&name) || image_queue::METHODS.contains(&name) || execution_plan::METHODS.contains(&name)
}

pub(super) fn schemas() -> Vec<Value> {
    serde_json::from_str(include_str!("../mcp-tools.json")).expect("build tool schemas")
}

pub(super) async fn capabilities() -> Json<Value> {
    let tools: Vec<_> = METHODS.iter().chain(video_queue::METHODS).chain(project_authoring::METHODS)
        .chain(canvas_write::METHODS).chain(capture_plan::METHODS).chain(managed_comfy::METHODS).chain(canvas_extensions::METHODS).chain(image_queue::METHODS).chain(final_export::METHODS).chain(execution_plan::METHODS).collect();
    let definitions: Vec<_> = schemas().into_iter().filter(|tool| tool["name"].as_str().is_some_and(native_method)).collect();
    Json(json!({ "tools": tools, "toolDefinitions": definitions, "stdioOnlyTools":stdio_mcp::STDIO_ONLY_TOOLS.map(|(name,_)| name),
        "toolRevision": revision_string(&json!(definitions).to_string()),
        "guideRevision": revision_string(&GUIDE.replace("\r\n", "\n")), "version": env!("CARGO_PKG_VERSION") }))
}

pub(super) async fn invoke(State(app): State<App>, call: std::result::Result<Json<Call>, JsonRejection>) -> Result<Json<Value>> {
    let Json(call) = call.map_err(|error| Failure::new(error.status(), error.body_text()))?;
    if call.name == "studio_extract_tail_frame" {
        return Ok(Json(canvas_write::extract_tail(&app, &call.input).await?));
    }
    if call.name == "studio_crop_shot_frame" {
        return Ok(Json(image_queue::crop_first_frame(&app,&call.input).await?));
    }
    if execution_plan::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(execution_plan::call_plan(&app,&call.name,&call.input)?));
    }
    if capture_plan::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(capture_plan::invoke(&app, &call.name, &call.input)?));
    }
    if final_export::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(final_export::call(&app,&call.name,&call.input)?));
    }
    if image_queue::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(image_queue::call(&app, &call.name, &call.input)?));
    }
    if canvas_extensions::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(canvas_extensions::call(&app, &call.name, &call.input).await?));
    }
    if managed_comfy::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(managed_comfy::call(&app, &call.name, &call.input).await?));
    }
    if video_queue::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(video_queue::dispatch(&app, &call.name, &call.input).await?));
    }
    let plan_guard = if matches!(call.name.as_str(),"studio_save_exit"|"studio_delete_project"|"studio_remove_project_asset") {
        let guard = app.plan_active.lock().map_err(|_| invalid("执行计划锁不可用"))?;
        if call.name == "studio_save_exit" && guard.is_some() { return Err(invalid("执行计划仍在运行，请先暂停后续步骤")); }
        Some(guard)
    } else { None };
    let video_guard = if matches!(call.name.as_str(),"studio_save_exit"|"studio_remove_model"|"studio_delete_project"|"studio_remove_project_asset") || project_authoring::changes_shot_order(&call.name) {
        let guard = app.video_active.lock().map_err(|_| invalid("视频队列锁不可用"))?;
        if call.name == "studio_save_exit" && guard.is_some() { return Err(invalid("视频队列正在运行，请先暂停后续镜头")); }
        Some(guard)
    } else { None };
    let _lock = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    if project_authoring::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(project_authoring::invoke(&app, &call.name, &call.input, video_guard.as_ref().and_then(|guard| guard.as_ref()))?));
    }
    if canvas_write::METHODS.contains(&call.name.as_str()) {
        return Ok(Json(canvas_write::invoke(&app, &call.name, &call.input)?));
    }
    if !METHODS.contains(&call.name.as_str()) {
        return Err(missing("Web 工作区尚未提供此操作"));
    }
    let input = call.input;
    match call.name.as_str() {
        "studio_delete_project" => return Ok(Json(delete_project(&app,&input,plan_guard.as_ref().and_then(|guard|guard.as_ref()),
            video_guard.as_ref().and_then(|guard|guard.as_ref()), false)?)),
        "studio_remove_project_asset" => {
            let running = [plan_guard.as_ref().and_then(|guard|guard.as_ref()),video_guard.as_ref().and_then(|guard|guard.as_ref())]
                .into_iter().flatten().any(|(project,_)| input["projectId"] == project.as_str());
            return Ok(Json(workspace::remove_project_asset(&app,&input,running)?));
        }
        "studio_get_guide" => return Ok(Json(guide(&input)?)),
        "studio_list_skills" => return Ok(Json(studio_skills(&app, None, None)?)),
        "studio_read_skill" => {
            let mut skill = studio_skills(&app, Some(field(&input, "skillId")?), input["referencePath"].as_str())?;
            skill["usage"] = json!(SKILL_USAGE);
            return Ok(Json(skill));
        }
        "studio_list_models" => return Ok(Json(list_models(&app, field(&input, "kind")?)?)),
        "studio_remove_model" => return Ok(Json(remove_model(&app, &input)?)),
        "studio_describe_model" => return Ok(Json(describe_model(&app, &input)?)),
        "studio_add_model" | "studio_select_model" | "studio_model_update" | "studio_set_credential" | "studio_patch_workflow" =>
            return Ok(Json(change_model(&app, &call.name, &input)?)),
        "studio_project" => {
            let active = active_project(&app)?;
            if input.get("projectId").is_some() {
                let id = field(&input, "projectId")?;
                if !active.as_ref().is_some_and(|(_, project)| project["id"] == id) {
                    return Err(missing("请先打开指定项目"));
                }
            }
            let project = active.map(|(file, project)| -> Result<Value> {
                let mut summary = project_meta(&project);
                if let Some(path) = file.parent().map(|folder| folder.join(project_authoring::PROJECT_INSTRUCTIONS)) {
                    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                        summary["projectInstructionsPath"] = json!(path);
                    }
                }
                let mut issues = Vec::new();
                for (kind, key) in [("image", "effectiveImage"), ("video", "effectiveVideo")] {
                    summary[key] = match default_model(&app, kind) {
                        Ok(model) => model,
                        Err(error) => {
                            issues.push(json!({"kind":kind,"issue":error.payload()}));
                            Value::Null
                        }
                    };
                }
                if !issues.is_empty() { summary["modelConfigurationIssues"] = json!(issues); }
                Ok(summary)
            }).transpose()?;
            return Ok(Json(json!({ "project":project })));
        },
        "studio_list_projects" => {
            let projects = each_project(&app)?.into_iter().map(|(_, project)| project_meta(&project)).collect::<Vec<_>>();
            return Ok(Json(paged(&projects, &input)));
        }
        "studio_open_project" => return Ok(Json(set_active_project(&app, field(&input, "projectId")?)?)),
        _ => {}
    }
    if call.name == "studio_save_exit" && input.get("projectId").is_none() {
        if input["confirmed"] != true || active_project(&app)?.is_some() {
            return Err(Failure::new(StatusCode::CONFLICT, "已有打开的项目或未确认退出".into()));
        }
        app.shutdown.send_replace(true);
        return Ok(Json(json!({ "exitRequested": true })));
    }
    let id = field(&input, "projectId")?;
    let (file, mut project) = active_project(&app)?.filter(|(_, value)| value["id"].as_str() == Some(id))
        .ok_or_else(|| missing("请先打开指定项目"))?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    let mut answer = match call.name.as_str() {
        "studio_save_exit" => {
            if input["confirmed"] != true {
                return Err(Failure::new(StatusCode::CONFLICT, "未确认退出".into()));
            }
            validate_project_revision(&input, revision, false)?;
            app.shutdown.send_replace(true);
            json!({ "exitRequested": true })
        }
        "studio_read_script" => {
            let chapter = if let Some(id) = input["chapterId"].as_str() {
                Some(array(&project["chapters"]).iter().find(|item| item["id"] == id)
                    .ok_or_else(|| missing("找不到指定章节"))?)
            } else { None };
            let script = &project["script"];
            let content = chapter.unwrap_or(script)["content"].as_str();
            let text = content.unwrap_or_default().encode_utf16().collect::<Vec<_>>();
            let offset = input["offset"].as_u64().unwrap_or(0).min(text.len() as u64) as usize;
            let limit = input["limit"].as_u64().unwrap_or(8000).clamp(1, 16000) as usize;
            let end = (offset + limit).min(text.len());
            json!({
                "topic": script["topic"], "keywords": array(&script["keywords"]), "style": script["style"],
                "aspectRatio": script["aspectRatio"], "playApproved": project["playApproved"] == true,
                "playReady": script["playReady"].as_bool().unwrap_or_else(|| script["origin"] == "imported" && script["content"].as_str().is_some_and(|text| !text.trim().is_empty())),
                "chapterId": chapter.map(|value| &value["id"]), "chapterTitle": chapter.map(|value| &value["title"]),
                "content": String::from_utf16_lossy(&text[offset..end]), "nextOffset": (end < text.len()).then_some(end)
            })
        }
        "studio_shots" | "studio_project_overview" => {
            let compact = call.name == "studio_project_overview";
            let chapter = input["chapterId"].as_str();
            let scene = input["sceneId"].as_str();
            let detail = if compact { "overview" } else { input["detail"].as_str().unwrap_or("full") };
            if !matches!(detail,"summary"|"overview"|"full") { return Err(invalid("分镜 detail 须为 summary、overview 或 full")); }
            let fields = input.get("fields").map(|fields| fields.as_array().filter(|items| !items.is_empty() && items.len() <= 30)
                .ok_or_else(|| invalid("fields 须为 1 至 30 个分镜字段"))).transpose()?;
            if input.get("shotIndices").is_some() || input.get("shotNumber").is_some() {
                return Err(invalid("请通过 shotId 或 shotIds 传项目全局镜号，例如 #05"));
            }
            let selected = match (input.get("shotId"),input.get("shotIds")) {
                (Some(id),None) => vec![project_authoring::resolve_shot_id(&project,
                    id.as_str().ok_or_else(|| invalid("shotId 须为镜头 ID 或 #05 格式的镜号"))?)?],
                (None,Some(ids)) => ids.as_array().filter(|ids| (1..=50).contains(&ids.len()))
                    .ok_or_else(|| invalid("shotIds 须包含 1 至 50 个镜头 ID 或镜号"))?.iter()
                    .map(|id| project_authoring::resolve_shot_id(&project,
                        id.as_str().ok_or_else(|| invalid("shotIds 中每项须为镜头 ID 或 #05 格式的镜号"))?))
                    .collect::<Result<Vec<_>>>()?,
                (Some(_),Some(_)) => return Err(invalid("shotId 与 shotIds 只能填写其一")),
                (None,None) => vec![],
            };
            let filtered: Vec<_> = project_authoring::timed_shots(&project).enumerate().filter(|(_, (shot, _, _))|
                chapter.is_none_or(|id| shot["chapterId"].as_str() == Some(id)) &&
                scene.is_none_or(|id| shot["sceneId"].as_str() == Some(id)) &&
                (selected.is_empty() || selected.iter().any(|id| shot["id"] == id.as_str()))
            ).collect();
            let offset = input["offset"].as_u64().unwrap_or(0).min(filtered.len() as u64) as usize;
            let default_limit = if detail == "overview" { 50 } else { 20 };
            let limit = input["limit"].as_u64().unwrap_or(default_limit).clamp(1, 50) as usize;
            let end = (offset + limit).min(filtered.len());
            let overview = detail == "overview" || fields.is_some_and(|fields| fields.iter().any(|key|
                matches!(key.as_str(),Some("hasImagePrompt"|"hasVideoPrompt"|"hasImage"|"hasVideo"|"boundSubjects"))));
            let assets = if overview { manifest(&app,&file)? } else { Value::Null };
            let exists = |id: &Value, kind: &str| id.as_str().is_some_and(|id|
                array(&assets).iter().find(|asset| asset["id"] == id && asset["kind"] == kind)
                    .and_then(|asset| asset["fileRelPath"].as_str())
                    .and_then(|name| file.parent().and_then(|root| scoped_path(&app.root,&root.join(name)).ok()))
                    .is_some_and(|media| media.is_file()));
            let items: Vec<_> = filtered[offset..end].iter().map(|(order,(shot,start,end))| {
                let mut item = if detail != "full" && fields.is_none() { summarize_shot(shot,*start,*end) } else { (*shot).clone() };
                let assigned = |key: &str| shot[key].as_str().is_some_and(|id| !id.trim().is_empty());
                item["status"] = json!(if assigned("videoAssetId") { "video-generated" }
                    else if assigned("imageAssetId") || assigned("boardAssetId") || assigned("firstFrameAssetId") { "image-generated" }
                    else { "draft" });
                item["shotNumber"] = json!(order+1);
                item["imageConsistency"] = json!(project_authoring::image_consistency(shot));
                item["plannedStartSec"] = json!(start);
                item["plannedEndSec"] = json!(end);
                if let Some(title) = shot["title"].as_str() {
                    item["displayTitle"] = json!(project_authoring::timed_title(title,order+1,*start,*end).as_deref().unwrap_or(title));
                }
                if overview {
                    let present = |key: &str| shot[key].as_str().is_some_and(|body| !body.trim().is_empty());
                    item["hasImagePrompt"] = json!(present("imagePrompt") || present("visualPrompt"));
                    item["hasVideoPrompt"] = json!(present("compiledVideoPrompt"));
                    item["hasImage"] = json!(exists(&shot["imageAssetId"],"image"));
                    item["hasVideo"] = json!(exists(&shot["videoAssetId"],"video"));
                    for key in ["imageAssetId","boardAssetId","firstFrameAssetId","lastFrameAssetId","videoAssetId","videoProfileId"] {
                        item[key] = shot[key].clone();
                    }
                    let ids = array(&shot["subjectIds"]);
                    let bound_subjects: Vec<_> = ids.iter().map(|id| {
                        let subject = array(&project["subjects"]).iter().find(|subject| subject["id"] == *id);
                        let name = subject.and_then(|subject| subject["name"].as_str());
                        let has_reference = subject.is_some_and(|subject| exists(&subject["designSheetAssetId"],"image"));
                        json!({"id":id,"name":name,"hasReference":has_reference})
                    }).collect();
                    item["subjectIds"] = json!(ids);
                    item["boundSubjects"] = json!(bound_subjects);
                    item["subjectBindingMode"] = json!(shot["subjectBindingMode"].as_str().unwrap_or("infer"));
                }
                if compact {
                    let subjects: Vec<_> = array(&item["boundSubjects"]).iter().map(|subject|
                        subject["name"].as_str().unwrap_or_else(|| subject["id"].as_str().unwrap_or_default())).collect();
                    return json!({"shotNumber":item["shotNumber"],"duration":item["duration"],
                        "hasPrompt":item["hasVideoPrompt"],"hasImagePrompt":item["hasImagePrompt"],
                        "hasImage":item["hasImage"],"hasVideo":item["hasVideo"],
                        "subjects":subjects,"plannedStart":item["plannedStartSec"]});
                }
                if let Some(fields) = fields {
                    let mut pick = json!({"id":shot["id"],"shotNumber":order+1});
                    for key in fields.iter().filter_map(Value::as_str) { pick[key] = item[key].clone(); }
                    pick
                } else { item }
            }).collect();
            json!({"items":items,"total":filtered.len(),"nextOffset":(end < filtered.len()).then_some(end)})
        },
        "studio_project_items" => {
            let kind = field(&input, "type")?;
            let page = match kind {
                "assets" => paged(array(&manifest(&app, &file)?), &input),
                "nodes" | "edges" => paged(array(&project["canvas"][kind]), &input),
                "subjects" | "scenes" | "props" => paged(array(&project[kind]), &input),
                "chapters" => {
                    let chapters = array(&project[kind]).iter().map(|value| json!({
                        "id": value["id"], "title": value["title"], "order": value["order"], "status": value["status"]
                    })).collect::<Vec<_>>();
                    paged(&chapters, &input)
                }
                _ => return Err(invalid("项目列表类型无效")),
            };
            let mut page = page;
            page["type"] = json!(kind);
            page
        }
        "studio_locate_asset" => {
            let id = field(&input, "assetId")?;
            let assets = manifest(&app, &file)?;
            let asset = array(&assets).iter().find(|item| item["id"].as_str() == Some(id))
                .ok_or_else(|| missing("找不到指定素材"))?;
            let full_path = asset["fileRelPath"].as_str().and_then(|path| file.parent().map(|dir| dir.join(path)))
                .and_then(|path| scoped_path(&app.root, &path).ok()).filter(|path| path.is_file());
            json!({ "assetId": id, "shotId": asset["shotId"], "kind": asset["kind"], "exists": full_path.is_some(), "path": full_path })
        }
        "studio_set_video_prompt" => {
            validate_project_revision(&input, revision, true)?;
            let result = project_authoring::set_video_prompt(&mut project,&input)?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |delta| delta.as_millis() as u64);
            project["updatedAt"] = json!(now.max(revision.saturating_add(1)));
            atomic_save(&file, &serde_json::to_vec_pretty(&project).map_err(|_| invalid("无法保存项目"))?)?;
            result
        }
        _ => unreachable!(),
    };
    answer["projectId"] = json!(id);
    answer["updatedAt"] = project["updatedAt"].clone();
    Ok(Json(answer))
}

pub(super) async fn browser_workspace(State(app): State<App>, Json(call): Json<Call>) -> Result<Json<Value>> {
    if call.name == "studio_delete_project" {
        let plan = app.plan_active.lock().map_err(|_| invalid("执行计划锁不可用"))?;
        let video = app.video_active.lock().map_err(|_| invalid("制作队列锁不可用"))?;
        let _write = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
        return Ok(Json(delete_project(&app,&call.input,plan.as_ref(),video.as_ref(),true)?));
    }
    if !matches!(call.name.as_str(),"studio_remove_model" | "studio_remove_shot" | "studio_reorder_shots" | "studio_describe_model" | "studio_update_shot_inputs") {
        return Err(invalid("此操作未向工作台开放"));
    }
    invoke(State(app),Ok(Json(call))).await
}
