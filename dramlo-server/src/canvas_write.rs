use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const METHODS: &[&str] = &[
    "studio_canvas_node_add",
    "studio_canvas_node_update",
    "studio_canvas_node_remove",
    "studio_extract_tail_frame",
];

/// 只有这四类节点由调用方自由创建，其余节点由项目数据决定：
/// 人物/场景/道具走 studio_upsert_project_assets，镜头走 studio_append_shots，
/// 首尾帧、视频、合成由生成与导出流程管理。这里不接受它们，避免出现
/// 数据里没有、画布上却存在的孤儿节点。
const FREE_TYPES: &[&str] = &["imageGen", "videoGen", "audioGen", "compose"];

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 传了 projectId 就按 id 找，否则用当前打开的项目。与其它写入工具一致。
fn current_project(app: &App, input: &Value) -> Result<(PathBuf, Value)> {
    if let Some(id) = input["projectId"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
    {
        return studio_mcp::project_by_id(app, id);
    }
    studio_mcp::active_project(app)?.ok_or_else(|| invalid("当前没有打开的项目"))
}

fn text_field<'a>(value: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    let text = value[key]
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| invalid("缺少必要的正文"))?;
    if text.encode_utf16().count() > limit {
        return Err(invalid("正文过长"));
    }
    Ok(text)
}

fn canvas_nodes(project: &mut Value) -> Result<&mut Vec<Value>> {
    project["canvas"]["nodes"]
        .as_array_mut()
        .ok_or_else(|| invalid("项目画布格式无效"))
}

fn free_node_index(nodes: &[Value], id: &str) -> Result<usize> {
    let index = nodes
        .iter()
        .position(|node| node["id"].as_str() == Some(id))
        .ok_or_else(|| invalid("找不到指定画布节点"))?;
    let kind = nodes[index]["type"].as_str().unwrap_or("");
    if !FREE_TYPES.contains(&kind) {
        return Err(invalid(
            "只能操作自由节点（imageGen / videoGen / audioGen / compose），流水线节点由项目数据生成",
        ));
    }
    Ok(index)
}

/// 新节点放在最右一列的下方，一列满四个再往右开一列。
/// 早期版本一律放到画布最右侧，节点多了会横着排成一条长带。
fn next_position(nodes: &[Value]) -> Value {
    let free: Vec<&Value> = nodes
        .iter()
        .filter(|node| {
            node["type"]
                .as_str()
                .is_some_and(|kind| FREE_TYPES.contains(&kind))
        })
        .collect();
    if free.is_empty() {
        let max_x = nodes
            .iter()
            .filter_map(|node| node["position"]["x"].as_f64())
            .fold(0.0_f64, f64::max);
        return json!({ "x": max_x + 340.0, "y": 120.0 });
    }
    let column_x = free
        .iter()
        .filter_map(|node| node["position"]["x"].as_f64())
        .fold(f64::NEG_INFINITY, f64::max);
    let in_column = |node: &&&Value| {
        node["position"]["x"]
            .as_f64()
            .is_some_and(|x| (x - column_x).abs() < 1.0)
    };
    if free.iter().filter(in_column).count() >= 4 {
        return json!({ "x": column_x + 340.0, "y": 120.0 });
    }
    let max_y = free
        .iter()
        .filter(in_column)
        .filter_map(|node| node["position"]["y"].as_f64())
        .fold(0.0_f64, f64::max);
    json!({ "x": column_x, "y": max_y + 280.0 })
}

fn add(app: &App, input: &Value) -> Result<Value> {
    let (file, mut project) = current_project(app, input)?;
    let revision = project["updatedAt"]
        .as_u64()
        .ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;

    let kind = input["type"].as_str().unwrap_or("");
    if !FREE_TYPES.contains(&kind) {
        return Err(invalid(
            "type 只能是 imageGen、videoGen、audioGen 或 compose",
        ));
    }
    let prompt = if kind == "compose" {
        input["prompt"].as_str().unwrap_or("").trim().to_string()
    } else {
        text_field(input, "prompt", 8000)?.to_string()
    };

    // 只有调用方显式指定画幅才写进节点；不传就留空，读取侧回退项目默认。
    // 以前这里把项目默认抄进每个节点，导致改项目画幅后这些节点不跟随。
    let aspect = input["aspectRatio"].as_str().map(str::to_string);

    let nodes = canvas_nodes(&mut project)?;
    // 合成是整片唯一的出口，界面上也是复用同一个节点。已有就返回它，不再建第二个。
    if kind == "compose" {
        if let Some(existing) = nodes.iter().find(|node| node["type"] == "compose") {
            let existing_id = existing["id"].clone();
            let project_id = project["id"].clone();
            return Ok(json!({
                "projectId": project_id, "nodeId": existing_id, "type": kind,
                "reused": true, "updatedAt": revision
            }));
        }
    }
    let count = nodes.len();
    // 同一毫秒内删了再建时，毫秒相同、节点数也可能相同，靠循环保证唯一。
    let mut node_id = format!("{}_{}_{}", kind, now(), count);
    while nodes.iter().any(|node| node["id"].as_str() == Some(&node_id)) {
        node_id.push('0');
    }
    let position = if input["position"].is_object() {
        json!({
            "x": input["position"]["x"].as_f64().unwrap_or(0.0),
            "y": input["position"]["y"].as_f64().unwrap_or(0.0)
        })
    } else {
        next_position(nodes)
    };

    let mut data = json!({ "label": input["label"].as_str().unwrap_or("") });
    if kind != "compose" {
        data["prompt"] = json!(prompt);
        if let Some(explicit) = &aspect {
            data["aspectRatio"] = json!(explicit);
        }
    }
    if kind == "videoGen" {
        let duration = input["duration"].as_f64().unwrap_or(5.0);
        if !(1.0..=30.0).contains(&duration) {
            return Err(invalid("duration 须在 1 到 30 秒之间"));
        }
        data["duration"] = json!(duration);
    }
    if data["label"] == "" {
        data["label"] = json!(match kind {
            "imageGen" => "图片",
            "videoGen" => "视频",
            "audioGen" => "口播",
            _ => "合成",
        });
    }

    let source = input["sourceNodeId"].as_str();
    if let Some(source_id) = source {
        if !nodes.iter().any(|node| node["id"].as_str() == Some(source_id)) {
            return Err(invalid("sourceNodeId 指向的节点不存在"));
        }
    }

    let node = json!({
        "id": node_id,
        "type": kind,
        "position": position,
        "data": data,
        "status": "idle"
    });
    nodes.push(node);

    let mut edge_id = Value::Null;
    if let Some(source_id) = source {
        let edges = project["canvas"]["edges"]
            .as_array_mut()
            .ok_or_else(|| invalid("项目画布格式无效"))?;
        let exists = edges.iter().any(|edge| {
            edge["source"].as_str() == Some(source_id) && edge["target"].as_str() == Some(&node_id)
        });
        if !exists {
            let id = format!("e-{}-{}", source_id, node_id);
            edges.push(json!({
                "id": id, "source": source_id, "target": node_id,
                "sourceHandle": "out", "targetHandle": "in"
            }));
            edge_id = json!(id);
        }
    }

    let revision = now().max(revision.saturating_add(1));
    project["updatedAt"] = json!(revision);
    save_project(&file, &project)?;
    notify_project_revision(app, json!({ "projectId": project["id"], "updatedAt": revision }));
    Ok(json!({
        "projectId": project["id"], "nodeId": node_id, "type": kind,
        "position": position, "edgeId": edge_id, "updatedAt": revision
    }))
}

fn update(app: &App, input: &Value) -> Result<Value> {
    let (file, mut project) = current_project(app, input)?;
    let revision = project["updatedAt"]
        .as_u64()
        .ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;

    let node_id = input["nodeId"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| invalid("缺少 nodeId"))?;

    let mut changed = Vec::new();
    // 素材存在性检查要在借用 canvas 之前做。
    let attach_asset = if let Some(asset_id) = input["assetId"].as_str() {
        if asset_id.is_empty() {
            Some(None)
        } else {
            let assets = studio_mcp::manifest(app, &file)?;
            let exists = assets
                .as_array()
                .into_iter()
                .flatten()
                .any(|asset| asset["id"].as_str() == Some(asset_id));
            if !exists {
                return Err(invalid(
                    "素材不存在。先用 studio_import_image_asset 导入，再挂到节点上",
                ));
            }
            Some(Some(asset_id.to_string()))
        }
    } else {
        None
    };
    {
        let nodes = canvas_nodes(&mut project)?;
        let index = free_node_index(nodes, node_id)?;
        let node = &mut nodes[index];

        if let Some(prompt) = input["prompt"].as_str() {
            if prompt.encode_utf16().count() > 8000 {
                return Err(invalid("正文过长"));
            }
            node["data"]["prompt"] = json!(prompt);
            changed.push("prompt");
        }
        if let Some(label) = input["label"].as_str() {
            if label.encode_utf16().count() > 120 {
                return Err(invalid("节点名过长"));
            }
            node["data"]["label"] = json!(label);
            changed.push("label");
        }
        if input["position"].is_object() {
            node["position"] = json!({
                "x": input["position"]["x"].as_f64().unwrap_or(0.0),
                "y": input["position"]["y"].as_f64().unwrap_or(0.0)
            });
            changed.push("position");
        }
        if let Some(duration) = input["duration"].as_f64() {
            if !(1.0..=30.0).contains(&duration) {
                return Err(invalid("duration 须在 1 到 30 秒之间"));
            }
            node["data"]["duration"] = json!(duration);
            changed.push("duration");
        }
        if let Some(asset) = attach_asset {
            match asset {
                Some(id) => {
                    node["data"]["outputAssetId"] = json!(id);
                    node["status"] = json!("success");
                }
                None => {
                    node["data"]["outputAssetId"] = Value::Null;
                    node["status"] = json!("idle");
                }
            }
            changed.push("assetId");
        }
    }
    if changed.is_empty() {
        return Err(invalid("没有要修改的字段"));
    }

    let revision = now().max(revision.saturating_add(1));
    project["updatedAt"] = json!(revision);
    save_project(&file, &project)?;
    notify_project_revision(app, json!({ "projectId": project["id"], "updatedAt": revision }));
    Ok(json!({
        "projectId": project["id"], "nodeId": node_id,
        "changed": changed, "updatedAt": revision
    }))
}

fn remove(app: &App, input: &Value) -> Result<Value> {
    let (file, mut project) = current_project(app, input)?;
    let revision = project["updatedAt"]
        .as_u64()
        .ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;

    let node_id = input["nodeId"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| invalid("缺少 nodeId"))?;

    let removed_edges: usize;
    let mut cleared_refs = 0_usize;
    {
        let nodes = canvas_nodes(&mut project)?;
        let index = free_node_index(nodes, node_id)?;
        nodes.remove(index);
        // 其它节点把这个节点当作参考来源时一并移除，否则会留下悬空引用。
        for node in nodes.iter_mut() {
            let Some(list) = node["data"]["referenceSourceNodeIds"].as_array_mut() else {
                continue;
            };
            if list.iter().any(|id| id.as_str() == Some(node_id)) {
                list.retain(|id| id.as_str() != Some(node_id));
                cleared_refs += 1;
            }
        }
        let edges = project["canvas"]["edges"]
            .as_array_mut()
            .ok_or_else(|| invalid("项目画布格式无效"))?;
        let before = edges.len();
        edges.retain(|edge| {
            edge["source"].as_str() != Some(node_id) && edge["target"].as_str() != Some(node_id)
        });
        removed_edges = before - edges.len();
    }

    let revision = now().max(revision.saturating_add(1));
    project["updatedAt"] = json!(revision);
    save_project(&file, &project)?;
    notify_project_revision(app, json!({ "projectId": project["id"], "updatedAt": revision }));
    Ok(json!({
        "projectId": project["id"], "nodeId": node_id,
        "removedEdges": removed_edges, "clearedReferences": cleared_refs, "updatedAt": revision
    }))
}

pub(super) async fn extract_tail(app: &App, input: &Value) -> Result<Value> {
    let (file, project) = current_project(app, input)?;
    let revision = project["updatedAt"]
        .as_u64()
        .ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, true)?;

    if input["nodeId"].as_str().is_some_and(|id|!id.trim().is_empty()) ==
        input["videoAssetId"].as_str().is_some_and(|id|!id.trim().is_empty()) {
        return Err(invalid("请在 videoAssetId 与 nodeId 中选一个视频来源"));
    }
    let video_id = if let Some(node_id) = input["nodeId"]
        .as_str()
        .filter(|id| !id.trim().is_empty())
    {
        let node = project["canvas"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|node| node["id"].as_str() == Some(node_id))
            .ok_or_else(|| invalid("找不到指定画布节点"))?;
        node["data"]["outputAssetId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("这个节点还没有生成视频"))?
            .to_owned()
    } else {
        input["videoAssetId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("请提供 videoAssetId 或 nodeId"))?
            .to_owned()
    };

    let folder = file.parent().ok_or_else(|| invalid("项目目录无效"))?;
    let video = video_queue::media_path(app, &file, &video_id, "video")?;
    let frames = video_queue::last_frame(&video).await?;
    let png = fs::read(frames.path().join("tail.png"))?;
    let project_id = project["id"].clone();

    // 素材清单由生图归档和截帧共用，写入要互斥，否则并发时后写的会丢掉前一条。
    let _guard = app
        .writes
        .lock()
        .map_err(|_| invalid("项目写锁不可用"))?;
    let (current_file, latest) = current_project(app, input)?;
    let current_revision = latest["updatedAt"].as_u64();
    if file != current_file || current_revision != Some(revision) {
        return Err(Failure::revision_conflict(current_revision, Some(revision)));
    }
    if video_queue::media_path(app, &file, &video_id, "video")? != video {
        return Err(invalid("原视频已变更，请重新选择"));
    }
    let manifest_path = scoped_path(folder, &folder.join("assets/manifest.json"))?;
    let mut manifest: Vec<Value> = serde_json::from_slice(&fs::read(&manifest_path)?)
        .map_err(|_| invalid("资产清单无效"))?;
    // 同一毫秒内多次截帧会撞 id，顺延编号。
    let stamp = now();
    let mut asset = format!("frame_{stamp}");
    let mut seq = 1;
    while manifest
        .iter()
        .any(|item| item["id"].as_str() == Some(asset.as_str()))
    {
        asset = format!("frame_{stamp}_{seq}");
        seq += 1;
    }
    let filename = format!("{}.png", asset);
    let directory = scoped_path(folder, &folder.join("assets/images"))?;
    fs::create_dir_all(&directory)?;
    let mut handle =
        tempfile::Builder::new().prefix("tail_").suffix(".png").tempfile_in(&directory)?;
    use std::io::Write;
    handle.write_all(&png)?;
    handle.as_file().sync_all()?;

    manifest.push(json!({
        "id": asset, "kind": "image", "filename": filename, "mimeType": "image/png",
        "size": png.len(), "createdAt": stamp, "source": "extracted",
        "projectId": project_id, "fileRelPath": format!("assets/images/{filename}"),
        "derivedFromAssetId": video_id
    }));
    let serialized =
        serde_json::to_vec_pretty(&manifest).map_err(|_| invalid("无法保存素材清单"))?;
    let saved = handle.persist_noclobber(directory.join(&filename)).map_err(|error| error.error)?;
    drop(saved);
    if let Err(error) = atomic_save(&manifest_path, &serialized) {
        let _ = fs::remove_file(directory.join(&filename));
        return Err(error);
    }

    Ok(json!({
        "projectId": project_id, "assetId": asset, "sourceVideoAssetId": video_id,
        "bytes": png.len(), "updatedAt": revision
    }))
}

pub(super) fn invoke(app: &App, name: &str, input: &Value) -> Result<Value> {
    match name {
        "studio_canvas_node_add" => add(app, input),
        "studio_canvas_node_update" => update(app, input),
        "studio_canvas_node_remove" => remove(app, input),
        _ => Err(invalid("未知的画布操作")),
    }
}

fn save_project(path: &Path, value: &Value) -> Result<()> {
    atomic_save(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| invalid("无法保存项目"))?,
    )
}
