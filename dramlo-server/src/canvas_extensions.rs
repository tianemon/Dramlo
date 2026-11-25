use super::*;
use serde::Serialize;
use base64::{Engine, engine::general_purpose::STANDARD};
use studio_mcp::missing;

pub(super) const METHODS: &[&str] = &[
    "studio_list_node_types", "studio_add_extension_node", "studio_update_extension_node", "studio_describe_extension_node", "studio_connect_extension_node", "studio_run_extension_node",
    "studio_prepare_comfy_node", "studio_extension_library", "studio_extension_preview", "studio_extension_install", "studio_extension_enable", "studio_extension_remove",
    "studio_extension_package_preview", "studio_extension_package_install", "studio_extension_package_export", "studio_call_extension_tool", "studio_describe_extension_tool",
];

const TAIL: &str = "studio.video-tail";
const MCP: &str = "studio.mcp";
const COMFY: &str = "studio.comfy-video";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Parameter {
    key: String,
    label: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<Value>,
    #[serde(default, rename = "enum", skip_serializing_if = "Vec::is_empty")]
    choices: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<ParameterTarget>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParameterTarget {
    node_id: String,
    field: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Port {
    id: String,
    kind: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    id: String,
    version: u32,
    name: String,
    description: String,
    executor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    params: Vec<Parameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inputs: Option<Vec<Port>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<Port>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Installed {
    manifest: Manifest,
    enabled_projects: Vec<String>,
    #[serde(default)]
    imported: bool,
}

fn builtin() -> Manifest {
    Manifest { format_version: 1, id: TAIL.into(), version: 1, name: "提取视频尾帧".into(),
        description: "从项目视频素材或上游视频节点的当前输出提取最后一帧。".into(), executor: TAIL.into(), profile_id: None, provider_id: None, tool: None, output_kind: None,
        params: vec![], inputs: None, outputs: None }
}

fn installed(app: &App) -> Result<Vec<Installed>> {
    match fs::read(kv_path(app, "canvas-extensions")?) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("扩展库记录无效")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn using_video_profile(app: &App, id: &str) -> Result<Vec<Value>> {
    Ok(installed(app)?.into_iter().filter(|entry| entry.manifest.profile_id.as_deref() == Some(id))
        .map(|entry| json!({"typeId":entry.manifest.id,"version":entry.manifest.version,"enabledProjects":entry.enabled_projects})).collect())
}

fn ports(manifest: &Manifest, input: bool) -> Vec<Port> {
    let declared = if input { &manifest.inputs } else { &manifest.outputs };
    if let Some(ports) = declared { return ports.clone(); }
    let default = match (manifest.executor.as_str(), input) {
        (TAIL, true) => Some(("video", "video")),
        (TAIL, false) => Some(("image", "image")),
        (COMFY, false) => Some(("out", "video")),
        (MCP, false) if manifest.output_kind.as_deref() == Some("image") => Some(("image", "image")),
        _ => None,
    };
    default.into_iter().map(|(id, kind)| Port { id:id.into(), kind:kind.into() }).collect()
}

fn definition(manifest: &Manifest, source: &str, enabled: bool) -> Value {
    json!({ "id": manifest.id, "version": manifest.version, "source": source, "enabled": enabled,
        "name": manifest.name, "description": manifest.description, "executor": manifest.executor,
        "inputs": ports(manifest, true), "outputs": ports(manifest, false), "params": manifest.params,
        "tool": manifest.tool, "providerId": manifest.provider_id })
}

fn catalog(app: &App, input: &Value, include_disabled: bool) -> Result<Value> {
    let project = input["projectId"].as_str();
    if let Some(project) = project { studio_mcp::project_by_id(app, project)?; }
    let mut items = vec![definition(&builtin(), "built-in", true)];
    for entry in installed(app)? {
        let enabled = project.is_some_and(|project| entry.enabled_projects.iter().any(|id| id == project));
        if enabled || include_disabled {
            let mut item = definition(&entry.manifest, if entry.imported { "imported" } else { "custom" }, enabled);
            if include_disabled { item["manifest"] = json!(entry.manifest); }
            items.push(item);
        }
    }
    Ok(json!({ "items": items }))
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 80
        && value.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
        && !matches!(value, "__proto__" | "constructor" | "prototype")
}

fn model_filename(value: &str) -> bool {
    value.len() <= 512 && !value.contains([':', '\0'])
        && value.split(['/', '\\']).all(|part| !part.trim().is_empty() && !matches!(part.trim(), "." | ".."))
        && [".safetensors", ".ckpt", ".gguf", ".onnx", ".pth", ".pt", ".bin"]
            .iter().any(|suffix| value.to_ascii_lowercase().ends_with(suffix))
}

fn parameter_type(kind: &str, value: &Value) -> bool {
    match kind {
        "integer" => value.as_i64().is_some_and(|n| (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&n)),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "string" => value.as_str().is_some_and(|text| text.len() <= 4096),
        "model" => value.as_str().is_some_and(model_filename),
        _ => false,
    }
}

fn parameter_value(param: &Parameter, value: &Value) -> Result<()> {
    if !parameter_type(&param.kind, value) || (!param.choices.is_empty() && !param.choices.contains(value)) ||
        value.as_f64().is_some_and(|n| param.min.is_some_and(|min| n < min) || param.max.is_some_and(|max| n > max)) {
        return Err(invalid(&format!("扩展参数 {} 不符合类型、范围或可选值", param.key)));
    }
    Ok(())
}

fn effective_params(manifest: &Manifest, overrides: &Value) -> Result<Value> {
    let mut values = serde_json::Map::new();
    for param in &manifest.params {
        if let Some(value) = &param.default { values.insert(param.key.clone(), value.clone()); }
    }
    if !overrides.is_null() {
        for (key, value) in overrides.as_object().ok_or_else(|| invalid("扩展参数必须为对象"))? {
            let param = manifest.params.iter().find(|param| param.key == *key)
                .ok_or_else(|| invalid(&format!("未声明扩展参数 {key}")))?;
            parameter_value(param, value)?;
            values.insert(key.clone(), value.clone());
        }
    }
    Ok(Value::Object(values))
}

fn validate_declarations(manifest: &Manifest) -> Result<()> {
    let mut keys = std::collections::HashSet::new();
    if manifest.params.len() > 32 || (manifest.executor == TAIL && !manifest.params.is_empty()) {
        return Err(invalid("参数数量超限或执行器不支持参数"));
    }
    for param in &manifest.params {
        let numeric = matches!(param.kind.as_str(), "integer" | "number");
        if !identifier(&param.key) || !keys.insert(&param.key) || param.label.trim().is_empty() || param.label.chars().count() > 80 ||
            !matches!(param.kind.as_str(), "integer" | "number" | "boolean" | "string" | "model") ||
            (!numeric && (param.min.is_some() || param.max.is_some())) ||
            param.min.zip(param.max).is_some_and(|(min,max)| min > max) ||
            param.choices.len() > 100 || (manifest.executor == COMFY) != param.target.is_some() {
            return Err(invalid("扩展参数声明无效"));
        }
        for choice in &param.choices { parameter_value(param, choice)?; }
        if let Some(value) = &param.default { parameter_value(param, value)?; }
    }
    let inputs = ports(manifest, true);
    let outputs = ports(manifest, false);
    let (max_inputs, expected_input) = match manifest.executor.as_str() {
        TAIL => (1, "video"), COMFY => (10, "image"), _ => (0, ""),
    };
    let expected_output = match manifest.executor.as_str() {
        TAIL => Some("image"), COMFY => Some("video"), MCP => manifest.output_kind.as_deref(), _ => None,
    };
    let mut port_ids = std::collections::HashSet::new();
    if inputs.len() > max_inputs || inputs.iter().any(|port| port.kind != expected_input || !identifier(&port.id) || !port_ids.insert(&port.id)) ||
        outputs.len() != usize::from(expected_output.is_some()) ||
        outputs.iter().any(|port| Some(port.kind.as_str()) != expected_output || !identifier(&port.id)) {
        return Err(invalid("端口类型或数量与执行器不兼容"));
    }
    Ok(())
}

fn apply_parameters(manifest: &Manifest, profile: &Value, graph: &mut Value, values: &Value) -> Result<()> {
    let bindings = video_queue::workflow_bindings(profile, graph);
    let mut targets = std::collections::HashSet::new();
    for param in &manifest.params {
        let Some(target) = &param.target else { continue; };
        let kind = graph[&target.node_id]["class_type"].as_str().unwrap_or_default().to_ascii_lowercase();
        let current = &graph[&target.node_id]["inputs"][&target.field];
        if !targets.insert((&target.node_id, &target.field)) ||
            bindings.iter().any(|binding| binding["nodeId"] == target.node_id && binding["field"] == target.field) ||
            kind.contains("loadimage") || !parameter_type(&param.kind, current) ||
            (param.kind == "string" && param.choices.is_empty()) {
            return Err(invalid(&format!("参数 {} 的目标必须是独立的同类型标量，不能覆盖生成映射或连线", param.key)));
        }
        if let Some(value) = values.get(&param.key) {
            graph[&target.node_id]["inputs"][&target.field] = value.clone();
        }
    }
    Ok(())
}

fn checked_manifest(app: &App, input: &Value) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_value(input["manifest"].clone())
        .map_err(|_| invalid("扩展清单字段无效"))?;
    let parts: Vec<_> = manifest.id.split('.').collect();
    if manifest.format_version != 1 || manifest.version == 0 ||
        !(2..=5).contains(&parts.len()) || manifest.id.len() > 80 || parts[0] == "studio" ||
        parts.iter().any(|part| part.is_empty() || !part.bytes().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')) ||
        manifest.name.trim().is_empty() || manifest.name.chars().count() > 80 ||
        manifest.description.trim().is_empty() || manifest.description.chars().count() > 500 {
        return Err(invalid("清单须使用格式 1、非 studio 命名空间及受支持的执行器"));
    }
    if manifest.output_kind.is_some() && (manifest.executor != MCP || manifest.output_kind.as_deref() != Some("image")) {
        return Err(invalid("可选的 MCP 输出类型只支持 image"));
    }
    validate_declarations(&manifest)?;
    match (manifest.executor.as_str(), manifest.profile_id.as_deref(), manifest.provider_id.as_deref(), manifest.tool.as_deref()) {
        (TAIL, None, None, None) => {}
        (COMFY, Some(profile), None, None) => {
            let profile = video_queue::get_profile(app, Some(profile))?;
            if profile["adapter"]["type"] != "comfyui-prompt" {
                return Err(invalid("请选择已导入的原生 ComfyUI 视频配置"));
            }
            if !manifest.params.is_empty() {
                let mut graph = video_queue::workflow_template(&profile)?;
                apply_parameters(&manifest, &profile, &mut graph, &effective_params(&manifest, &Value::Null)?)?;
            }
        }
        (MCP, None, Some(provider), Some(tool)) => { extension_stdio::resolve(app, provider, tool)?; }
        _ => return Err(invalid("执行器或配置无效")),
    }
    Ok(manifest)
}

fn library_write(app: &App, name: &str, input: &Value) -> Result<Value> {
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let mut entries = installed(app)?;
    if name == "studio_extension_remove" {
        if input["confirmed"] != true { return Err(invalid("请先确认卸载扩展")); }
        let id = field(input, "typeId")?;
        let version = input["version"].as_u64().ok_or_else(|| invalid("请选择扩展版本"))?;
        let count = entries.len();
        entries.retain(|entry| entry.manifest.id != id || u64::from(entry.manifest.version) != version);
        let removed = entries.len() != count;
        if removed {
            atomic_save(&kv_path(app, "canvas-extensions")?, &serde_json::to_vec(&entries)
                .map_err(|_| invalid("无法保存扩展库"))?)?;
        }
        return Ok(json!({ "removed": removed }));
    }
    let result = if matches!(name, "studio_extension_preview" | "studio_extension_install") {
        let manifest = checked_manifest(app, input)?;
        let existing = entries.iter().find(|entry| entry.manifest.id == manifest.id && entry.manifest.version == manifest.version);
        if existing.is_some_and(|entry| entry.manifest != manifest) { return Err(invalid("同 ID 和版本已有不同内容")); }
        let result = definition(&manifest, "custom", false);
        if name == "studio_extension_preview" || existing.is_some() { return Ok(result); }
        entries.push(Installed { manifest, enabled_projects: vec![], imported: false });
        result
    } else {
        let project = field(input, "projectId")?;
        studio_mcp::project_by_id(app, project)?;
        let id = field(input, "typeId")?;
        let version = input["version"].as_u64().ok_or_else(|| invalid("请选择扩展版本"))?;
        let enabled = input["enabled"].as_bool().ok_or_else(|| invalid("请选择启用或停用"))?;
        let entry = entries.iter_mut().find(|entry| entry.manifest.id == id && u64::from(entry.manifest.version) == version)
            .ok_or_else(|| missing("找不到自定义扩展版本"))?;
        entry.enabled_projects.retain(|item| item != project);
        if enabled { entry.enabled_projects.push(project.into()); }
        definition(&entry.manifest, if entry.imported { "imported" } else { "custom" }, enabled)
    };
    atomic_save(&kv_path(app, "canvas-extensions")?, &serde_json::to_vec(&entries).map_err(|_| invalid("无法保存扩展库"))?)?;
    Ok(result)
}

fn package(app: &App, name: &str, input: &Value) -> Result<Value> {
    if name == "studio_extension_package_export" {
        let id = field(input, "typeId")?;
        let version = input["version"].as_u64().ok_or_else(|| invalid("请选择扩展版本"))?;
        let entry = installed(app)?.into_iter().find(|entry| entry.manifest.id == id && u64::from(entry.manifest.version) == version)
            .ok_or_else(|| missing("找不到扩展版本"))?;
        return extension_package::export(&json!(entry.manifest));
    }
    let document = extension_package::read(input)?;
    let candidates = if let Some(entries) = document.get("extensions") {
        if document["formatVersion"] != 1 || document.as_object().map(|obj| obj.len()) != Some(2) {
            return Err(invalid("扩展包清单格式无效"));
        }
        entries.as_array().filter(|items| !items.is_empty() && items.len() <= 16)
            .cloned().ok_or_else(|| invalid("扩展包须包含 1 至 16 条清单"))?
    } else { vec![document] };
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let mut installed = installed(app)?;
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        let manifest = checked_manifest(app, &json!({"manifest":candidate}))?;
        if !seen.insert((manifest.id.clone(), manifest.version)) { return Err(invalid("扩展包内有重复 ID 和版本")); }
        let existing = installed.iter().find(|item| item.manifest.id == manifest.id && item.manifest.version == manifest.version);
        if existing.is_some_and(|entry| entry.manifest != manifest) { return Err(invalid("同 ID 和版本已有不同内容")); }
        items.push(definition(&manifest, "imported", false));
        if existing.is_none() { installed.push(Installed { manifest, enabled_projects: vec![], imported: true }); }
    }
    if name == "studio_extension_package_install" {
        atomic_save(&kv_path(app, "canvas-extensions")?, &serde_json::to_vec(&installed).map_err(|_| invalid("无法保存扩展库"))?)?;
    }
    Ok(json!({ "items": items }))
}

fn resolve(app: &App, project: &str, id: &str, version: u64) -> Result<Manifest> {
    if id == TAIL && version == 1 { return Ok(builtin()); }
    installed(app)?.into_iter().find(|entry| entry.manifest.id == id && u64::from(entry.manifest.version) == version &&
        entry.enabled_projects.iter().any(|item| item == project)).map(|entry| entry.manifest)
        .ok_or_else(|| missing("扩展未安装、已停用或版本不匹配"))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().filter(|s| !s.is_empty()).ok_or_else(|| invalid("缺少节点操作字段"))
}

fn nodes(project: &Value) -> &[Value] {
    project["canvas"]["nodes"].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn node<'a>(project: &'a Value, id: &str) -> Result<&'a Value> {
    nodes(project).iter().find(|node| node["id"] == id).ok_or_else(|| missing("找不到画布节点"))
}

fn video_node(node: &Value) -> bool {
    matches!(node["type"].as_str(), Some("videoGen" | "shotVideo")) ||
        (node["type"] == "extension" && node["data"]["extensionExecutor"] == COMFY)
}

fn image_slot(node: &Value) -> Option<(&'static str, &'static str, &'static str)> {
    match node["type"].as_str()? {
        "subject" => Some(("subjects","subjectId","designSheetAssetId")),
        "subjectProps" => Some(("subjects","subjectId","propSheetAssetId")),
        "prop" => Some(("props","propId","designSheetAssetId")),
        "scene" => Some(("scenes","sceneId","plateAssetId")),
        "sceneProps" => Some(("scenes","sceneId","propSheetAssetId")),
        "shotBoard" => Some(("shots","shotId","boardAssetId")),
        "shotImage" => Some(("shots","shotId","imageAssetId")),
        "shotFrames" => Some(("shots","shotId","firstFrameAssetId")),
        _ => None,
    }
}

fn image_asset<'a>(project: &'a Value, node: &'a Value) -> Option<&'a str> {
    if let Some((collection,id,slot)) = image_slot(node) {
        let record = project[collection].as_array()?.iter().find(|record| record["id"] == node["data"][id])?;
        record[slot].as_str().or_else(|| (node["type"] == "shotFrames").then(|| record["lastFrameAssetId"].as_str()).flatten())
    } else { node["data"]["outputAssetId"].as_str() }
}

fn extension_node(app: &App, project: &str, node: &Value) -> Result<Manifest> {
    if node["type"] != "extension" { return Err(invalid("请选择扩展节点")); }
    resolve(app, project, field(&node["data"], "extensionType")?,
        node["data"]["extensionVersion"].as_u64().ok_or_else(|| invalid("画布节点缺少扩展版本"))?)
}

fn tail_input(project: &Value, target: &Value, manifest: &Manifest) -> Result<String> {
    let inputs = ports(manifest, true);
    if let Some(edge) = project["canvas"]["edges"].as_array().into_iter().flatten()
        .find(|edge| edge["target"] == target["id"] && inputs.iter().any(|port| edge["targetHandle"] == port.id)) {
        let source = node(project, field(edge, "source")?)?;
        if !video_node(source) { return Err(invalid("尾帧节点只能连接视频节点")); }
        if source["type"] == "shotVideo" {
            let shot = project["shots"].as_array().into_iter().flatten()
                .find(|shot| shot["id"] == source["data"]["shotId"]).ok_or_else(|| missing("找不到视频分镜"))?;
            return Ok(field(shot, "videoAssetId").map_err(|_| invalid("上游分镜尚未选用视频"))?.to_owned());
        }
        return Ok(field(&source["data"], "outputAssetId").map_err(|_| invalid("上游节点尚无视频输出"))?.to_owned());
    }
    Ok(field(&target["data"], "extensionVideoAssetId").map_err(|_| invalid("请连接视频节点或选择视频素材"))?.to_owned())
}

fn write(app: &App, input: &Value, update: impl FnOnce(&Path, &mut Value) -> Result<Value>) -> Result<Value> {
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let (path, mut project) = studio_mcp::project_by_id(app, field(input, "projectId")?)?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;
    let mut result = update(&path, &mut project)?;
    let revision = video_queue::now().max(project["updatedAt"].as_u64().unwrap_or_default().saturating_add(1));
    project["updatedAt"] = json!(revision);
    atomic_save(&path, &serde_json::to_vec_pretty(&project).map_err(|_| invalid("无法保存项目"))?)?;
    result["updatedAt"] = json!(revision);
    Ok(result)
}

fn list<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Vec<Value>> {
    if value[key].is_null() { value[key] = json!([]); }
    value[key].as_array_mut().ok_or_else(|| invalid("画布格式无效"))
}

fn add(app: &App, input: &Value) -> Result<Value> {
    let type_id = field(input, "typeId")?;
    let version = input["version"].as_u64().unwrap_or(1);
    let id = field(input, "nodeId")?;
    let video = input["videoAssetId"].as_str();
    let x = input["x"].as_f64().unwrap_or(0.0);
    let y = input["y"].as_f64().unwrap_or(0.0);
    write(app, input, |path, project| {
        let manifest = resolve(app, field(project, "id")?, type_id, version)?;
        if nodes(project).iter().any(|node| node["id"] == id) { return Err(invalid("节点 ID 已存在")); }
        if let Some(video) = video { video_queue::media_path(app, path, video, "video")?; }
        let mut entry = json!({ "id": id, "type": "extension", "position": { "x": x, "y": y },
            "data": { "extensionType": manifest.id, "extensionVersion": manifest.version, "extensionExecutor": manifest.executor, "label": manifest.name, "extensionVideoAssetId": video }, "status": "idle" });
        if let Some(values) = input.get("params") {
            if !values.is_object() { return Err(invalid("扩展参数必须为对象")); }
            effective_params(&manifest, values)?;
            entry["data"]["extensionParams"] = values.clone();
        }
        list(&mut project["canvas"], "nodes")?.push(entry.clone());
        Ok(json!({ "node": entry }))
    })
}

async fn describe_node(app: &App, input: &Value) -> Result<Value> {
    let (_, project) = studio_mcp::project_by_id(app, field(input, "projectId")?)?;
    let entry = node(&project, field(input, "nodeId")?)?;
    let manifest = extension_node(app, field(&project, "id")?, entry)?;
    let mut result = json!({"nodeId":entry["id"],"updatedAt":project["updatedAt"],"definition":definition(&manifest,"project",true),
        "overrides":entry["data"]["extensionParams"],"params":effective_params(&manifest,&entry["data"]["extensionParams"])?});
    if input["includeChoices"] == true && manifest.executor == COMFY {
        let profile = video_queue::get_profile(app, manifest.profile_id.as_deref())?;
        let graph = video_queue::workflow_template(&profile)?;
        video_queue::endpoint(&profile)?;
        let address = field(&profile,"baseUrl")?.trim_end_matches('/');
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6);
        let mut info = std::collections::HashMap::new();
        let mut options = serde_json::Map::new();
        for param in manifest.params.iter().filter(|param| matches!(param.kind.as_str(),"model" | "string")) {
            let Some(target) = &param.target else { continue; };
            let class = field(&graph[&target.node_id],"class_type")?;
            if !info.contains_key(class) {
                let route = format!("/object_info/{}", video_queue::escape(class));
                let data = if tokio::time::Instant::now() < deadline {
                    tokio::time::timeout_at(deadline, comfy_mcp::get_at(address,&route)).await.ok().and_then(Result::ok)
                } else { None };
                info.insert(class,data);
            }
            options.insert(param.key.clone(),info[class].as_ref().map_or(Value::Null,|data| {
                let inputs = &data[class]["input"];
                let values = inputs["required"][&target.field].as_array().or_else(|| inputs["optional"][&target.field].as_array())
                    .and_then(|items| items.first()).and_then(Value::as_array);
                values.map_or(Value::Null,|items| {
                    let mut choices = items.iter().filter(|value| parameter_value(param,value).is_ok());
                    let values = choices.by_ref().take(1000).collect::<Vec<_>>();
                    json!({"items":values,"truncated":choices.next().is_some()})
                })
            }));
        }
        result["choices"] = Value::Object(options);
    }
    Ok(result)
}

fn update_node(app: &App, input: &Value) -> Result<Value> {
    let id = field(input, "nodeId")?;
    let changes = input["params"].as_object().filter(|obj| !obj.is_empty())
        .ok_or_else(|| invalid("请提供要修改的扩展参数"))?;
    write(app, input, |_, project| {
        let entry = node(project, id)?;
        let manifest = extension_node(app, field(project, "id")?, entry)?;
        let mut values = entry["data"]["extensionParams"].as_object().cloned().unwrap_or_default();
        for (key, value) in changes {
            let param = manifest.params.iter().find(|param| param.key == *key)
                .ok_or_else(|| invalid(&format!("未声明扩展参数 {key}")))?;
            if value.is_null() { values.remove(key); }
            else {
                parameter_value(param, value)?;
                values.insert(key.clone(),value.clone());
            }
        }
        let values = Value::Object(values);
        let effective = effective_params(&manifest, &values)?;
        let entry = list(&mut project["canvas"], "nodes")?.iter_mut().find(|node| node["id"] == id)
            .ok_or_else(|| missing("找不到画布节点"))?;
        entry["data"]["extensionParams"] = values;
        Ok(json!({"nodeId":id,"params":effective}))
    })
}

fn choose_port(ports: &[Port], requested: &Value) -> Result<Port> {
    let selected = if let Some(id) = requested.as_str() {
        ports.iter().find(|port| port.id == id)
    } else if requested.is_null() && ports.len() == 1 { ports.first() } else { None };
    selected.cloned().ok_or_else(|| invalid("请指定有效的输入或输出端口 ID"))
}

fn connect(app: &App, input: &Value) -> Result<Value> {
    let source = field(input, "sourceId")?;
    let target = field(input, "targetId")?;
    write(app, input, |_, project| {
        let src = node(project, source)?;
        let dst = node(project, target)?;
        let project_id = field(project, "id")?;
        let source_ports = if src["type"] == "extension" { ports(&extension_node(app, project_id, src)?, false) }
            else if video_node(src) { vec![Port{id:"out".into(),kind:"video".into()}] }
            else if src["type"] == "imageGen" || image_slot(src).is_some() { vec![Port{id:"out".into(),kind:"image".into()}] }
            else { vec![] };
        let target_ports = if dst["type"] == "extension" { ports(&extension_node(app, project_id, dst)?, true) }
            else if dst["type"] == "videoGen" && src["type"] == "extension" {
                vec![Port{id:"first-frame".into(),kind:"image".into()}]
            } else { vec![] };
        let out = choose_port(&source_ports, &input["sourceHandle"])?;
        let to = choose_port(&target_ports, &input["targetHandle"])?;
        if out.kind != to.kind { return Err(invalid("输入和输出的素材类型不匹配")); }
        let edges = list(&mut project["canvas"], "edges")?;
        let mut pending = vec![target];
        let mut visited = std::collections::HashSet::new();
        while let Some(id) = pending.pop() {
            if id == source { return Err(invalid("连线不能形成循环")); }
            if visited.insert(id) {
                pending.extend(edges.iter().filter(|edge| edge["source"] == id)
                    .filter_map(|edge| edge["target"].as_str()));
            }
        }
        if edges.iter().any(|edge| edge["target"] == target && edge["targetHandle"] == to.id &&
            (edge["source"] != source || edge["sourceHandle"] != out.id)) {
            return Err(invalid("该输入端口已连接，请先断开旧连接"));
        }
        if let Some(edge) = edges.iter().find(|edge| edge["source"] == source && edge["target"] == target &&
            edge["sourceHandle"] == out.id && edge["targetHandle"] == to.id) {
            return Ok(json!({"edge":edge}));
        }
        let base = format!("e-{source}-{target}-{}", to.id);
        let id = (0..=edges.len()).map(|n| if n == 0 { base.clone() } else {format!("{base}-{n}")})
            .find(|id| !edges.iter().any(|edge| edge["id"] == *id)).ok_or_else(|| invalid("无法分配连线 ID"))?;
        let edge = json!({ "id": id, "source": source, "target": target,
            "sourceHandle": out.id, "targetHandle": to.id });
        edges.push(edge.clone());
        Ok(json!({ "edge": edge }))
    })
}

async fn call_external(app: &App, input: &Value, manifest: &Manifest) -> Result<Value> {
    let provider = manifest.provider_id.as_deref().ok_or_else(|| invalid("扩展未配置 MCP 连接"))?;
    let tool = manifest.tool.as_deref().ok_or_else(|| invalid("扩展未绑定工具"))?;
    let provider = extension_stdio::resolve(app, provider, tool)?;
    let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !arguments.is_object() || arguments.to_string().len() > 64 * 1024 {
        return Err(invalid("MCP 工具参数必须为 64 KB 以内的 JSON 对象"));
    }
    extension_stdio::call(&provider, tool, arguments).await
}

async fn external_tool(app: &App, input: &Value, describe: bool) -> Result<Value> {
    let project = field(input, "projectId")?;
    studio_mcp::project_by_id(app, project)?;
    let version = input["version"].as_u64().ok_or_else(|| invalid("请选择扩展版本"))?;
    let manifest = resolve(app, project, field(input, "typeId")?, version)?;
    if describe {
        let provider_id = manifest.provider_id.as_deref().ok_or_else(|| invalid("扩展未配置 MCP 连接"))?;
        let tool = manifest.tool.as_deref().ok_or_else(|| invalid("扩展未绑定工具"))?;
        let provider = extension_stdio::resolve(app, provider_id, tool)?;
        return extension_stdio::describe(&provider, tool).await;
    }
    call_external(app, input, &manifest).await
}

fn external_image(path: &Path, project: &Value, node_id: &str, result: &mut Value) -> Result<String> {
    let image = result["content"].as_array_mut().into_iter().flatten().find(|item| item["type"] == "image")
        .ok_or_else(|| invalid("上游工具没有返回声明的图片"))?;
    let mime = field(image, "mimeType")?.to_owned();
    let bytes = STANDARD.decode(field(image, "data")?).map_err(|_| invalid("上游图片编码无效"))?;
    let ext = match mime.as_str() {
        "image/png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "png",
        "image/jpeg" if bytes.starts_with(b"\xff\xd8\xff") => "jpg",
        "image/webp" if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") => "webp",
        _ => return Err(invalid("上游图片格式不匹配")),
    };
    let folder = path.parent().ok_or_else(|| invalid("项目目录无效"))?;
    let dir = scoped_path(folder, &folder.join("assets/images"))?;
    let manifest_path = scoped_path(folder, &folder.join("assets/manifest.json"))?;
    let mut assets: Vec<Value> = match fs::read(&manifest_path) {
        Ok(data) => serde_json::from_slice(&data).map_err(|_| invalid("素材清单无效"))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    fs::create_dir_all(&dir)?;
    let suffix = format!(".{ext}");
    let mut file = tempfile::Builder::new().prefix("plugin_").suffix(&suffix).tempfile_in(dir)?;
    use std::io::Write;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    let filename = file.path().file_name().and_then(|name| name.to_str()).ok_or_else(|| invalid("素材文件名无效"))?.to_owned();
    let id = filename.strip_suffix(&suffix).unwrap_or(&filename).to_owned();
    assets.push(json!({ "id":id,"kind":"image","filename":filename,"mimeType":mime,"size":bytes.len(),
        "createdAt":video_queue::now(),"source":"ai-generated","projectId":project["id"],
        "fileRelPath":format!("assets/images/{filename}"),"extensionNodeId":node_id }));
    let serialized = serde_json::to_vec_pretty(&assets).map_err(|_| invalid("无法保存素材清单"))?;
    let (_, saved_path) = file.keep().map_err(|error| error.error)?;
    if let Err(error) = atomic_save(&manifest_path, &serialized) {
        let _ = fs::remove_file(saved_path);
        return Err(error);
    }
    *image = json!({"type":"asset","assetId":id,"mimeType":mime});
    Ok(id)
}

async fn run(app: &App, input: &Value) -> Result<Value> {
    run_node(app,input,None).await
}

pub(super) fn inspect_tail_plan(app: &App, path: &Path, project: &Value, node_id: &str, source: Option<&str>, pending: bool) -> Result<Value> {
    let entry = node(project,node_id)?;
    let manifest = extension_node(app,field(project,"id")?,entry)?;
    if manifest.executor != TAIL { return Err(invalid("此计划步骤只支持提取视频尾帧节点")); }
    if super::ffmpeg_binary().is_err() {
        return Err(invalid("请配置 DRAMLO_FFMPEG_BIN 以截取视频尾帧"));
    }
    let asset = match source {
        Some(source) => source.to_owned(),
        None => tail_input(project,entry,&manifest)?,
    };
    if !pending { video_queue::media_path(app,path,&asset,"video")?; }
    Ok(json!({"approvalStamp":video_queue::stamp(&json!([node_id,entry["data"]["extensionTypeId"],entry["data"]["extensionVersion"],
        if source.is_some() {None} else {Some(&asset)}])),"sourceAssetId":if pending {None} else {Some(asset)}}))
}

pub(super) async fn run_tail_plan(app: &App, input: &Value, source: Option<&str>, request_id: &str) -> Result<Value> {
    run_node(app,input,Some((source,request_id))).await
}

async fn run_node(app: &App, input: &Value, tail_plan: Option<(Option<&str>, &str)>) -> Result<Value> {
    let id = field(input, "nodeId")?;
    let (path, project) = studio_mcp::project_by_id(app, field(input, "projectId")?)?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;
    // Async work must only save against the snapshot used to resolve the node and its inputs.
    let mut pinned = input.clone();
    pinned["expectedUpdatedAt"] = json!(revision);
    let input = &pinned;
    let entry = node(&project, id)?;
    let manifest = extension_node(app, field(&project, "id")?, entry)?;
    if tail_plan.is_some() && manifest.executor != TAIL { return Err(invalid("此计划步骤只支持提取视频尾帧节点")); }
    if manifest.executor == MCP {
        let arguments = if manifest.params.is_empty() { input.clone() } else {
            let mut values = effective_params(&manifest, &entry["data"]["extensionParams"])?;
            if let Some(arguments) = input.get("arguments") {
                for (key,value) in arguments.as_object().ok_or_else(|| invalid("MCP 工具参数必须为对象"))? {
                    values[key] = value.clone();
                }
            }
            json!({"arguments":effective_params(&manifest, &values)?})
        };
        let mut result = call_external(app, &arguments, &manifest).await?;
        return write(app, input, |path, project| {
            extension_node(app, field(project, "id")?, node(project, id)?)?;
            let asset_id = if manifest.output_kind.as_deref() == Some("image") {
                Some(external_image(path, project, id, &mut result)?)
            } else { None };
            let entry = list(&mut project["canvas"], "nodes")?.iter_mut().find(|node| node["id"] == id)
                .ok_or_else(|| missing("找不到画布节点"))?;
            entry["data"]["extensionResult"] = result.clone();
            if let Some(id) = &asset_id { entry["data"]["outputAssetId"] = json!(id); }
            entry["status"] = json!("success");
            Ok(json!({ "nodeId": id, "result": result, "assetId": asset_id }))
        }).map_err(|error| Failure::new(error.0, format!("上游调用已完成，画布未保存。请勿直接重试。{}", error.1)));
    }
    if manifest.executor != TAIL {
        return Err(invalid("请先预检并确认 ComfyUI 视频任务"));
    }
    let source = match tail_plan.and_then(|(asset,_)| asset) {
        Some(asset) => asset.to_owned(),
        None => tail_input(&project,entry,&manifest)?,
    };
    let video = video_queue::media_path(app, &path, &source, "video")?;
    let folder = path.parent().ok_or_else(|| invalid("项目目录无效"))?;
    if !video.canonicalize()?.starts_with(folder.canonicalize()?) { return Err(invalid("视频不在当前项目目录")); }
    let request = tail_plan.map(|(_,request)|request);
    let existing = request.and_then(|request| {
        let items = studio_mcp::read_json(&folder.join("assets/manifest.json")).ok()?;
        items.as_array()?.iter().find(|asset| asset["planRequestId"] == request && asset["derivedFromAssetId"] == source)
            .and_then(|asset| asset["id"].as_str()).map(str::to_owned)
    });
    let png = if existing.is_none() {
        let frames = video_queue::last_frame(&video).await?;
        Some(fs::read(frames.path().join("tail.png"))?)
    } else { None };
    write(app, input, |path, project| {
        extension_node(app, field(project, "id")?, node(project, id)?)?;
        if let Some(asset) = &existing {
            video_queue::media_path(app,path,asset,"image")?;
            let entry = list(&mut project["canvas"],"nodes")?.iter_mut().find(|node|node["id"]==id)
                .ok_or_else(|| missing("找不到画布节点"))?;
            entry["data"]["outputAssetId"] = json!(asset);
            entry["data"]["extensionSourceAssetId"] = json!(source);
            entry["status"] = json!("success");
            return Ok(json!({"nodeId":id,"assetId":asset}));
        }
        let png = png.as_deref().ok_or_else(||invalid("尾帧数据缺失"))?;
        let folder = path.parent().ok_or_else(|| invalid("项目目录无效"))?;
        let manifest_path = scoped_path(folder, &folder.join("assets/manifest.json"))?;
        let mut manifest: Vec<Value> = serde_json::from_slice(&fs::read(&manifest_path)?)
            .map_err(|_| invalid("资产清单无效"))?;
        let directory = scoped_path(folder, &folder.join("assets/images"))?;
        fs::create_dir_all(&directory)?;
        let mut file = tempfile::Builder::new().prefix("tail_").suffix(".png").tempfile_in(directory)?;
        use std::io::Write;
        file.write_all(png)?;
        let filename = file.path().file_name().and_then(|name| name.to_str()).ok_or_else(|| invalid("素材文件名无效"))?.to_owned();
        let asset = filename.trim_end_matches(".png");
        manifest.push(json!({ "id": asset, "kind": "image", "filename": filename, "mimeType": "image/png",
            "size": png.len(), "createdAt": video_queue::now(), "source": "extracted",
            "projectId": project["id"], "fileRelPath": format!("assets/images/{filename}"),
            "derivedFromAssetId": source, "extensionNodeId": id, "planRequestId":request }));
        let serialized = serde_json::to_vec_pretty(&manifest).map_err(|_| invalid("无法保存素材清单"))?;
        file.as_file().sync_all()?;
        let (_, saved_path) = file.keep().map_err(|error| error.error)?;
        if let Err(error) = atomic_save(&manifest_path, &serialized) {
            let _ = fs::remove_file(saved_path);
            return Err(error);
        }
        let entry = list(&mut project["canvas"], "nodes")?.iter_mut().find(|node| node["id"] == id)
            .ok_or_else(|| missing("找不到画布节点"))?;
        entry["data"]["outputAssetId"] = json!(asset);
        entry["data"]["extensionSourceAssetId"] = json!(source);
        entry["status"] = json!("success");
        Ok(json!({ "nodeId": id, "assetId": asset }))
    })
}

fn comfy_node(app: &App, project: &Value, node_id: &str, profile: &str) -> Result<Manifest> {
    let manifest = extension_node(app, field(project, "id")?, node(project, node_id)?)?;
    if manifest.executor != COMFY || manifest.profile_id.as_deref() != Some(profile) {
        return Err(invalid("ComfyUI 扩展或工作流配置已改变"));
    }
    Ok(manifest)
}

pub(super) fn comfy_state(app: &App, project: &Value, node_id: &str, profile: &str) -> Result<Option<Value>> {
    let manifest = comfy_node(app, project, node_id, profile)?;
    let entry = node(project, node_id)?;
    let params = effective_params(&manifest, &entry["data"]["extensionParams"])?;
    let mut images = Vec::new();
    let inputs = ports(&manifest, true);
    let edges = project["canvas"]["edges"].as_array().map(Vec::as_slice).unwrap_or_default();
    if inputs.iter().any(|port| edges.iter().any(|edge| edge["target"] == node_id && edge["targetHandle"] == port.id)) {
        for port in inputs {
            let mut links = edges.iter().filter(|edge| edge["target"] == node_id && edge["targetHandle"] == port.id);
            let edge = links.next().ok_or_else(|| invalid(&format!("请连接参考图端口 {}",port.id)))?;
            if links.next().is_some() { return Err(invalid("同一个参考图端口只能连接一张图片")); }
            let source = node(project,field(edge,"source")?)?;
            if source["type"] == "extension" {
                let upstream = extension_node(app,field(project,"id")?,source)?;
                if !ports(&upstream,false).iter().any(|port| port.kind == "image" && edge["sourceHandle"] == port.id) {
                    return Err(invalid("上游扩展没有对应的图片输出"));
                }
                if upstream.executor == TAIL && source["data"]["extensionSourceAssetId"] != tail_input(project,source,&upstream)? {
                    return Err(invalid("上游视频已变化，请重新提取尾帧"));
                }
            } else if (source["type"] != "imageGen" && image_slot(source).is_none()) || edge["sourceHandle"] != "out" {
                return Err(invalid("上游节点没有对应的图片输出"));
            }
            images.push(image_asset(project, source).ok_or_else(|| invalid("上游节点尚无图片，请先运行"))?);
        }
    }
    if images.is_empty() && params.as_object().is_some_and(|values| values.is_empty()) { return Ok(None); }
    Ok(Some(json!({"params":params,"referenceAssetIds":images})))
}

pub(super) fn apply_comfy_params(app: &App, project: &Value, node_id: &str, profile: &Value, graph: &mut Value, state: Option<&Value>) -> Result<()> {
    let manifest = comfy_node(app, project, node_id, field(profile,"id")?)?;
    apply_parameters(&manifest, profile, graph, state.map_or(&Value::Null, |value| &value["params"]))
}

async fn prepare_comfy(app: &App, input: &Value) -> Result<Value> {
    let id = field(input, "nodeId")?;
    let project_id = field(input, "projectId")?;
    let (_, project) = studio_mcp::project_by_id(app, project_id)?;
    let manifest = extension_node(app, project_id, node(&project, id)?)?;
    let profile = manifest.profile_id.as_deref().filter(|_| manifest.executor == COMFY)
        .ok_or_else(|| invalid("请选择已启用的 ComfyUI 扩展节点"))?;
    let mut take = input["shot"].clone();
    if !take.is_object() { return Err(invalid("请提供分镜及输入方式")); }
    take["profileId"] = json!(profile);
    take["extensionNodeId"] = json!(id);
    if input["dryRun"] == true {
        let mut probe = json!({"projectId":project_id,"shots":[take]});
        if let Some(revision) = input.get("expectedUpdatedAt") { probe["expectedUpdatedAt"] = revision.clone(); }
        let mut result = video_queue::dispatch(app,"studio_check_video_readiness",&probe).await?;
        result["dryRun"] = json!(true);
        return Ok(result);
    }
    let result = video_queue::dispatch(app, "studio_prepare_video", &json!({
        "projectId": project_id, "requestId": field(input, "requestId")?,
        "expectedUpdatedAt": input["expectedUpdatedAt"], "shots": [take]
    })).await?;
    if result["batch"]["steps"][0]["take"]["extensionNodeId"] != id {
        return Err(invalid("该请求 ID 已被其他画布任务使用"));
    }
    Ok(result)
}

pub(super) async fn call(app: &App, name: &str, input: &Value) -> Result<Value> {
    match name {
        "studio_extension_package_preview" | "studio_extension_package_install" | "studio_extension_package_export" => package(app, name, input),
        "studio_call_extension_tool" => external_tool(app, input, false).await,
        "studio_describe_extension_tool" => external_tool(app, input, true).await,
        "studio_prepare_comfy_node" => prepare_comfy(app, input).await,
        "studio_list_node_types" => catalog(app, input, false),
        "studio_extension_library" => catalog(app, input, true),
        "studio_extension_preview" | "studio_extension_install" | "studio_extension_enable" | "studio_extension_remove" => library_write(app, name, input),
        "studio_add_extension_node" => add(app, input),
        "studio_update_extension_node" => update_node(app, input),
        "studio_describe_extension_node" => describe_node(app, input).await,
        "studio_connect_extension_node" => connect(app, input),
        "studio_run_extension_node" => run(app, input).await,
        _ => Err(missing("未知画布扩展操作")),
    }
}

pub(super) async fn browser(State(app): State<App>, Json(request): Json<Value>) -> Result<Json<Value>> {
    let name = field(&request, "name")?;
    let input = &request["input"];
    let result = match name {
        "configure_mcp_provider" => extension_stdio::configure(&app, input)?,
        "list_mcp_providers" => extension_stdio::list(&app)?,
        "remove_mcp_provider" => extension_stdio::remove(&app, input)?,
        _ => call(&app, name, input).await?,
    };
    Ok(Json(result))
}
