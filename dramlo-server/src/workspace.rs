use super::*;

fn identifier(value: Option<&str>) -> Result<&str> {
    value.filter(|id| !id.is_empty() && id.len() <= 200 &&
        id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')))
        .ok_or_else(|| invalid("标识无效"))
}

fn references(value: &Value, id: &str) -> bool {
    match value {
        Value::String(value) => value == id,
        Value::Array(values) => values.iter().any(|value| references(value, id)),
        Value::Object(values) => values.values().any(|value| references(value, id)),
        _ => false,
    }
}

fn unfinished_references(app: &App, folder: &Path, project_id: &str, id: &str) -> Result<bool> {
    let plans = scoped_path(&app.root,&folder.join("execution-plans.json"))?;
    if plans.exists() {
        let plans = studio_mcp::read_json(&plans)?;
        let plans = plans.as_array().ok_or_else(|| invalid("执行计划记录无效"))?;
        if plans.iter().any(|plan| plan["state"] != "succeeded" && references(&plan["steps"],id)) { return Ok(true); }
    }
    for entry in fs::read_dir(state_directory(&app.root)?)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() || entry.path().extension().is_none_or(|ext| ext != "json") { continue; }
        let Ok(job) = studio_mcp::read_json(&entry.path()) else { continue; };
        if job["projectId"] != project_id { continue; }
        let outstanding = matches!(job["status"].as_str(), Some("running" | "starting" | "submitting" | "waiting" | "paused" | "unknown"))
            || matches!(job["state"].as_str(), Some("running" | "paused" | "unknown"));
        if outstanding && references(&job,id) { return Ok(true); }
    }
    Ok(false)
}

pub(super) fn remove_project_asset(app: &App, input: &Value, running: bool) -> Result<Value> {
    let fields = input.as_object().ok_or_else(|| invalid("请求必须是对象"))?;
    if let Some(field) = fields.keys().find(|key| !matches!(key.as_str(), "projectId" | "assetId" | "targetType" | "expectedUpdatedAt" | "confirmed")) {
        return Err(invalid(&format!("不支持字段「{field}」")));
    }
    let project_id = identifier(input["projectId"].as_str())?;
    let id = identifier(input["assetId"].as_str())?;
    let (file, mut project) = studio_mcp::project_by_id(app,project_id)?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    let folder = file.parent().ok_or_else(|| invalid("缺少项目目录"))?;
    let index = folder.join("assets/manifest.json");
    let target = match input.get("targetType") {
        None => "media",
        Some(value) => value.as_str().filter(|kind| matches!(*kind,"media"|"subject")).ok_or_else(|| invalid("目标类型无效"))?,
    };
    let entry = if target == "media" {
        studio_mcp::manifest(app,&file)?.as_array().into_iter().flatten().find(|item| item["id"] == id).cloned()
    } else {
        let entry = project["subjects"].as_array().and_then(|items| items.iter().find(|item| item["id"] == id)).cloned();
        if let Some(subjects) = project["subjects"].as_array_mut() { subjects.retain(|item| item["id"] != id); }
        let mut removed = std::collections::BTreeSet::new();
        if let Some(nodes) = project["canvas"]["nodes"].as_array_mut() {
            nodes.retain(|node| {
                if !matches!(node["type"].as_str(),Some("subject"|"subjectProps")) || node["data"]["subjectId"] != id { return true; }
                if let Some(node_id) = node["id"].as_str() { removed.insert(node_id.to_owned()); }
                false
            });
        }
        let removed_node = |value: &Value| value.as_str().is_some_and(|id| removed.contains(id));
        if let Some(edges) = project["canvas"]["edges"].as_array_mut() {
            edges.retain(|edge| !removed_node(&edge["source"]) && !removed_node(&edge["target"]));
        }
        if let Some(groups) = project["canvas"]["groups"].as_array_mut() {
            for group in groups {
                if let Some(ids) = group["nodeIds"].as_array_mut() { ids.retain(|id| !removed_node(id)); }
            }
        }
        if removed.iter().any(|id| references(&project,id)) { return Ok(json!({
            "removed":false,"blocked":true,"reason":"角色画布节点仍被引用","projectId":project_id,"assetId":id,"targetType":target,"updatedAt":revision
        })); }
        entry
    }.ok_or_else(|| studio_mcp::missing("找不到指定的项目素材或角色卡"))?;
    let reason = if references(&project,id) { Some("项目仍引用此条目，请先解除绑定") }
        else if running || unfinished_references(app,folder,project_id,id)? { Some("项目有运行中的制作任务或仍引用此条目的未结任务") }
        else { None };
    let mut result = json!({"projectId":project_id,"assetId":id,"targetType":target,"name":entry["name"],
        "filename":entry["filename"],"removed":false,"blocked":reason.is_some(),"reason":reason,"updatedAt":revision});
    if input["confirmed"] != true || reason.is_some() { return Ok(result); }
    validate_project_revision(input,revision,false)?;
    if target == "media" {
        indexed_remove(app,folder,&index,id,false)?;
    } else {
        project["updatedAt"] = json!(video_queue::now().max(revision.saturating_add(1)));
        atomic_save(&file,&serde_json::to_vec_pretty(&project).map_err(|_| invalid("无法写入项目"))?)?;
        result["updatedAt"] = project["updatedAt"].clone();
    }
    result["removed"] = json!(true);
    Ok(result)
}

fn indexed_remove(app: &App, directory: &Path, index: &Path, id: &str, scripts: bool) -> Result<Value> {
    let (index, directory) = (scoped_path(&app.root, index)?, scoped_path(&app.root, directory)?);
    let mut entries: Vec<Value> = serde_json::from_slice(&fs::read(&index)?)
        .map_err(|_| invalid("素材或剧本清单无效"))?;
    let item = entries.iter().find(|item| item["id"] == id)
        .ok_or_else(|| studio_mcp::missing("索引中已没有该条目"))?;
    let mut paths = Vec::new();
    if scripts {
        paths.push(directory.join(format!("{id}.md")));
    } else {
        let relative = Path::new(item["fileRelPath"].as_str().ok_or_else(|| invalid("素材缺少相对路径"))?);
        if relative.is_absolute() || !relative.starts_with("assets") || relative.components().count() < 3 {
            return Err(invalid("素材位置不属于该目录"));
        }
        paths.push(directory.join(relative));
        paths.push(directory.join("thumbs").join(format!("{id}.jpg")));
    }
    let paths = paths.iter().map(|path| scoped_path(&directory, path)).collect::<Result<Vec<_>>>()?;
    let recovery = tempfile::Builder::new().prefix("removed-item-").tempdir_in(state_directory(&app.root)?)?;
    atomic_save(&recovery.path().join("recovery.json"),json!({"index":index,"entry":item,"paths":paths}).to_string().as_bytes())?;
    let mut moved = Vec::new();
    entries.retain(|item| item["id"] != id);
    let change = (|| -> Result<()> {
        for (index, path) in paths.into_iter().enumerate() {
            if !path.exists() { continue; }
            if !fs::symlink_metadata(&path)?.file_type().is_file() { return Err(invalid("删除目标不是普通文件")); }
            let name = path.file_name().ok_or_else(|| invalid("缺少文件名"))?;
            let target = recovery.path().join(format!("{index}-{}", name.to_string_lossy()));
            fs::rename(&path, &target)?;
            moved.push((path, target));
        }
        atomic_save(&index, &serde_json::to_vec_pretty(&entries).map_err(|_| invalid("无法更新清单"))?)
    })();
    if let Err(error) = change {
        let mut rolled_back = true;
        for (source, target) in moved.iter().rev() {
            if fs::rename(target, source).is_err() { rolled_back = false; }
        }
        if !rolled_back {
            let path = recovery.keep();
            return Err(invalid(&format!("移除失败且文件未能全部恢复，请查看 {}", path.display())));
        }
        return Err(error);
    }
    if !moved.is_empty() { let _ = recovery.keep(); }
    Ok(json!(entries))
}

pub(super) fn remove_indexed(app: &App, options: &Command) -> Result<Value> {
    let id = identifier(options.id.as_deref())?;
    let kind = options.name.as_deref().unwrap_or_default();
    match kind {
        "script" => {
            let dir = app.root.join("_scripts");
            indexed_remove(app, &dir, &dir.join("index.json"), id, true)
        }
        "publicAsset" => {
            let dir = app.root.join("_library");
            indexed_remove(app, &dir, &dir.join("assets/manifest.json"), id, false)
        }
        "projectAsset" => {
            let index = options.path(app)?;
            if !is_asset_manifest(&index) { return Err(invalid("只能移除项目素材清单中的条目")); }
            let dir = index.parent().and_then(Path::parent).ok_or_else(|| invalid("缺少项目目录"))?;
            let project = studio_mcp::read_json(&dir.join("project.json"))?;
            if references(&project,id) { return Err(invalid("该素材仍被项目引用，请先解除绑定")); }
            if let Some(project_id) = project["id"].as_str() {
                if unfinished_references(app,dir,project_id,id)? { return Err(invalid("该素材仍被未结任务引用")); }
            }
            indexed_remove(app, dir, &index, id, false)
        }
        _ => Err(invalid("未知资源类型")),
    }
}
