use super::*;

const LIMIT: u64 = 2 * 1024 * 1024;

pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({ "name": "comfy_workflows_list", "description": "发现授权目录与本机 ComfyUI 中的工作流",
            "inputSchema": { "type": "object", "properties": { "includeServer": { "type": "boolean", "default": true, "description": "默认同时列出在线实例的已存工作流；仅扫描授权目录时填 false。" } } } }),
        json!({ "name": "comfy_workflow_read", "description": "读取工作流格式、输入连线和权重，可返回 API JSON",
            "inputSchema": { "type": "object", "required": ["source", "relativePath"], "properties": {
                "source": {"enum": ["disk", "server"], "type": "string", "description": "沿用工作流列表返回的来源，disk 为授权目录，server 为实例的保存目录。"}, "folder": {"type": "integer", "minimum": 0, "description": "disk 来源使用列表返回的目录序号，从 0 开始。"},
                "relativePath": {"type": "string", "description": "列表返回的工作流相对路径，保留 .json 后缀。"}, "includeApiJson": {"type": "boolean", "description": "填 true 返回 API 图 JSON 字符串。"}, "checkInstalled": {"type": "boolean", "description": "填 true 只读核对节点与服务可枚举的权重。"}, "includeNodeInputs": {"type": "boolean", "description": "读取当前 ComfyUI 的节点输入类型、枚举、范围和单位。"}
            } } }),
    ]
}

fn roots() -> Result<Vec<PathBuf>> {
    let raw = env::var("DRAMLO_COMFY_WORKFLOW_DIRS").unwrap_or_else(|_| "[]".into());
    let roots: Vec<PathBuf> = serde_json::from_str(&raw).map_err(|_| invalid("工作流目录应为 JSON 列表"))?;
    if roots.len() > 32 || roots.iter().any(|path| !path.is_absolute()) {
        return Err(invalid("请使用工作流绝对目录"));
    }
    Ok(roots)
}

pub(super) fn format(raw: &Value) -> (&str, Option<&Value>) {
    let graph = raw.get("prompt").unwrap_or(raw);
    if raw["nodes"].is_array() && raw["links"].is_array() { return ("ui", None); }
    let ok = graph.as_object().is_some_and(|nodes| !nodes.is_empty() && nodes.values()
        .all(|node| node["class_type"].is_string() && node["inputs"].is_object()));
    if ok { ("api", Some(graph)) } else { ("unknown", None) }
}

pub(super) fn disk(root: &Path, relative: &Path) -> Result<Value> {
    if relative.is_absolute() || !relative.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
        return Err(invalid("工作流文件位置无效"));
    }
    let root = normalize_root(root)?;
    let path = scoped_path(&root, relative)?;
    let meta = fs::metadata(&path)?;
    if !meta.is_file() || meta.len() > LIMIT { return Err(invalid("工作流文件过大或不是文件")); }
    let data = fs::read(path)?;
    let data = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&data);
    serde_json::from_slice(data).map_err(|_| invalid("工作流不是有效 JSON"))
}

pub(super) fn scan(root: &Path, relative: &Path, folder: usize, remaining: &mut usize, depth: usize, items: &mut Vec<Value>) -> Result<()> {
    if depth > 5 || *remaining == 0 { return Ok(()); }
    for item in fs::read_dir(root.join(relative))? {
        if *remaining == 0 { break; }
        *remaining -= 1;
        let item = item?;
        let next = relative.join(item.file_name());
        let kind = item.file_type()?;
        if kind.is_dir() {
            let bounded = normalize_root(&item.path()).is_ok_and(|candidate| normalize_root(root).is_ok_and(|root| candidate.starts_with(root)));
            if bounded { scan(root, &next, folder, remaining, depth + 1, items)?; }
        }
        if kind.is_file() && next.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")) {
            if let Ok(raw) = disk(root, &next) {
                let (format, _) = format(&raw);
                if format != "unknown" {
                    items.push(json!({ "source": "disk", "folder": folder, "relativePath": next,
                        "name": item.file_name().to_string_lossy().as_ref(), "format": format }));
                }
            }
        }
    }
    Ok(())
}

async fn get(path: &str) -> Result<Value> {
    let base = env::var("DRAMLO_COMFY_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
    get_at(&base, path).await
}

pub(super) async fn get_at(base: &str, path: &str) -> Result<Value> {
    let profile = json!({"baseUrl": base, "adapter": {"type":"comfyui-prompt"}});
    let (status, _, bytes) = tokio::time::timeout(std::time::Duration::from_secs(6),
        video_queue::http(&profile, "GET", path, "application/json", Vec::new())).await
        .map_err(|_| invalid("ComfyUI 读取超时"))??;
    if !status.is_success() || bytes.len() as u64 > LIMIT {
        return Err(invalid("ComfyUI 未返回可用的工作流数据"));
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("ComfyUI 返回无效 JSON"))
}

pub(super) fn describe(graph: &Value) -> Value {
    let mut classes = std::collections::BTreeSet::new();
    let mut slots = Vec::new();
    let mut weights = Vec::new();
    let nodes = graph.as_object().unwrap();
    let mut output = "unknown";
    for (id, node) in nodes {
        let kind = node["class_type"].as_str().unwrap_or_default();
        classes.insert(kind);
        if kind.contains("VideoCombine") || kind.contains("SaveVideo") { output = "video"; }
        else if output == "unknown" && (kind.contains("SaveImage") || kind.contains("PreviewImage")) { output = "image"; }
        for (key, value) in node["inputs"].as_object().unwrap() {
            if value.as_str().is_some_and(|value| [".safetensors", ".ckpt", ".gguf", ".onnx", ".pth", ".pt"]
                .iter().any(|suffix| value.to_ascii_lowercase().ends_with(suffix))) {
                weights.push(json!({ "nodeId": id, "type": kind, "field": key, "filename": value }));
            }
            if key == "first_frame" || key == "last_frame" || key == "end_frame" || key.starts_with("ref_images.ref_image_") {
                if value.is_array() {
                    slots.push(json!({ "nodeId": id, "type": kind, "field": key, "role": key, "connectedFrom": value[0] }));
                }
            } else if key == "prompt" && value.is_array() {
                if let Some((source, field)) = prompt_source(graph, value) {
                    slots.push(json!({ "nodeId": source, "field": field, "role": "prompt" }));
                }
            } else if value.is_string() && (key == "prompt" || (key == "text" && kind.contains("TextEncode"))) {
                slots.push(json!({ "nodeId": id, "type": kind, "field": key, "role": "prompt" }));
            } else if matches!(key.as_str(), "width" | "height" | "length" | "duration" | "fps" | "aspect_ratio") && !value.is_array() {
                slots.push(json!({ "nodeId": id, "type": kind, "field": key, "role": key }));
            }
        }
    }
    let mut unique_slots = std::collections::BTreeMap::new();
    for mut slot in slots {
        let id = slot["nodeId"].as_str().unwrap_or_default().to_owned();
        let field = slot["field"].as_str().unwrap_or_default().to_owned();
        if slot["type"].is_null() {
            slot["type"] = nodes[&id]["class_type"].clone();
        }
        unique_slots.entry((id, field)).or_insert(slot);
    }
    let slots = unique_slots.into_values().collect::<Vec<_>>();
    json!({ "format": "api", "nodeCount": nodes.len(), "output": output, "classes": classes, "weights": weights, "candidateSlots": slots })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_lists_a_shared_prompt_source_once() {
        let graph = json!({
            "1": {"class_type":"PrimitiveStringMultiline","inputs":{"value":"a prompt"}},
            "2": {"class_type":"StringReplace","inputs":{"replace":["1",0]}},
            "3": {"class_type":"TextConsumer","inputs":{"prompt":["2",0]}},
            "4": {"class_type":"TextConsumer","inputs":{"prompt":["1",0]}},
            "5": {"class_type":"ComfyVideo","inputs":{"length":24}}
        });
        assert_eq!(describe(&graph)["candidateSlots"], json!([
            {"nodeId":"1","type":"PrimitiveStringMultiline","field":"value","role":"prompt"},
            {"nodeId":"5","type":"ComfyVideo","field":"length","role":"length"}
        ]));
    }
}

fn prompt_source<'a>(graph: &'a Value, link: &'a Value) -> Option<(&'a str, &'static str)> {
    let mut current = link;
    let mut seen = std::collections::HashSet::new();
    loop {
        let id = current[0].as_str()?;
        if !seen.insert(id) { return None; }
        let node = &graph[id];
        let fields = node["inputs"].as_object()?;
        for key in ["prompt", "text", "value"] {
            if fields.get(key).is_some_and(Value::is_string) { return Some((id, key)); }
        }
        let branch = match node["class_type"].as_str().unwrap_or_default() {
            "ComfySwitchNode" => Some(if fields.get("switch")?.as_bool()? { "on_true" } else { "on_false" }),
            "JsonExtractString" => Some("json_string"), "TextGenerateV2" => Some("prompt"), "StringReplace" => Some("replace"),
            _ => None,
        };
        current = if let Some(key) = branch { fields.get(key)? } else {
            let mut links = fields.values().filter(|value| value.is_array());
            let link = links.next()?;
            if links.next().is_some() { return None; }
            link
        };
    }
}

async fn installed(summary: &Value) -> Value {
    let classes = summary["classes"].as_array().map_or(&[][..], Vec::as_slice);
    let operation = async {
        let mut reports = Vec::new();
        let mut missing = Vec::new();
        for class in classes.iter().filter_map(Value::as_str).take(120) {
            let result = get(&format!("/object_info/{}", video_queue::escape(class))).await;
            let available = result.as_ref().ok().map(|result| result[class].is_object());
            reports.push(json!({ "type": class, "available": available }));
            if let Ok(value) = result {
                let input = &value[class]["input"];
                for weight in summary["weights"].as_array().into_iter().flatten().filter(|item| item["type"] == class) {
                    let field = weight["field"].as_str().unwrap_or_default();
                    let options = input["required"][field].as_array().or_else(|| input["optional"][field].as_array());
                    if let Some(options) = options.and_then(|items| items.first()).and_then(Value::as_array) {
                        if !options.contains(&weight["filename"]) { missing.push(weight.clone()); }
                    }
                }
            }
        }
        json!({ "nodes": reports, "missingWeights": missing, "partial": classes.len() > 120 })
    };
    tokio::time::timeout(std::time::Duration::from_secs(12), operation).await
        .unwrap_or_else(|_| json!({ "nodes": [], "missingWeights": [], "partial": true, "error": "节点校验超时" }))
}

pub(super) async fn node_catalog(base: &str, graph: &Value, summary: &Value) -> Value {
    use futures_util::{StreamExt, stream};

    let classes: Vec<_> = summary["classes"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    let truncated = classes.len()>60;
    let mut queries = stream::iter(classes.into_iter().take(60)).map(|class| async move {
        let path = format!("/object_info/{}",video_queue::escape(&class));
        let info = tokio::time::timeout(std::time::Duration::from_secs(3),get_at(base,&path)).await;
        let result = info.ok().and_then(Result::ok);
        let mut fields = serde_json::Map::new();
        let definition = result.as_ref().map(|info| &info[&class]).filter(|info| info.is_object());
        if let Some(info) = definition {
            for group in ["required","optional"] {
                for (name,source) in info["input"][group].as_object().into_iter().flatten() {
                    let Some(items) = source.as_array() else { continue; };
                    let options = items.first().and_then(Value::as_array);
                    let meta = items.get(1).unwrap_or(&Value::Null);
                    fields.insert(name.to_owned(),json!({
                        "required":group=="required","type": if options.is_some() { Some("enum") } else { items.first().and_then(Value::as_str) },
                        "options":options,"min":meta["min"],"max":meta["max"],"step":meta["step"],"unit":meta["unit"]
                    }));
                }
            }
        }
        (class,json!({"available":definition.is_some(),"inputs":fields}))
    }).buffer_unordered(6);
    let mut schema = serde_json::Map::new();
    let complete = tokio::time::timeout(std::time::Duration::from_secs(12),async {
        while let Some((class,value)) = queries.next().await { schema.insert(class,value); }
    }).await.is_ok();
    let nodes: Vec<_> = graph.as_object().into_iter().flatten().map(|(id,node)| {
        let candidates: Vec<_> = summary["candidateSlots"].as_array().into_iter().flatten()
            .filter(|candidate| candidate["nodeId"]==id.as_str()).collect();
        json!({"nodeId":id,"classType":node["class_type"],"suggestedRoles":candidates})
    }).collect();
    json!({"nodes":nodes,"schemaByClass":schema,
        "partial":!complete || truncated || schema.values().any(|row| row["available"] != true)})
}

pub(super) async fn call(name: &str, args: &Value) -> Result<Value> {
    let roots = roots()?;
    match name {
        "comfy_workflows_list" => {
            let mut items = Vec::new();
            let mut problems = Vec::new();
            for (index, root) in roots.iter().enumerate() {
                let mut remaining = 1000;
                if let Err(error) = scan(root, Path::new(""), index, &mut remaining, 0, &mut items) {
                    problems.push(json!({ "root": root, "reason": error.1 }));
                } else if remaining == 0 { problems.push(json!({ "root": root, "reason": "超过扫描上限" })); }
            }
            if args["includeServer"] != false {
                match get("/userdata?dir=workflows&recurse=true&split=false&full_info=true").await {
                    Ok(value) => for item in value.as_array().into_iter().flatten().take(500) {
                        if let Some(item) = item.as_str().or_else(|| item["path"].as_str()).filter(|value| value.ends_with(".json")) {
                            let relative = item.strip_prefix("workflows/").unwrap_or(item);
                            items.push(json!({ "source": "server", "relativePath": relative, "name": relative.rsplit('/').next() }));
                        }
                    },
                    Err(error) => problems.push(json!({ "reason": error.1 }))
                }
            }
            Ok(json!({ "roots": roots, "items": items, "problems": problems }))
        }
        "comfy_workflow_read" => {
            let path = args["relativePath"].as_str().filter(|path| !path.is_empty() && path.len() < 2048)
                .ok_or_else(|| invalid("需要工作流相对路径"))?;
            let raw = match args["source"].as_str() {
                Some("disk") => {
                    let index = args["folder"].as_u64().and_then(|value| usize::try_from(value).ok()).unwrap_or(0);
                    disk(roots.get(index).ok_or_else(|| invalid("工作流目录序号无效"))?, Path::new(path))?
                }
                Some("server") => {
                    let path = path.replace('\\', "/");
                    if !path.ends_with(".json") || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
                        return Err(invalid("工作流路径无效"));
                    }
                    get(&format!("/userdata/{}", video_queue::escape(&format!("workflows/{path}")))).await?
                }
                _ => return Err(invalid("工作流来源无效")),
            };
            let (kind, embedded) = format(&raw);
            let converted;
            let graph = if let Some(graph) = embedded { graph }
                else if kind == "ui" && args["includeApiJson"] == true &&
                    raw["nodes"].as_array().is_some_and(|nodes| !nodes.is_empty()) {
                    let address = env::var("DRAMLO_COMFY_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
                    match managed_comfy::convert_ui(&address, &raw).await {
                        Ok(graph) => {
                            converted = graph;
                            format(&converted).1.ok_or_else(|| invalid("未取得可执行的 ComfyUI API 图"))?
                        }
                        Err(error) => return Ok(json!({ "format": "ui", "conversionError": error.1 })),
                    }
                } else { return Ok(json!({ "format": kind, "detail": "非空界面图可请求自动转换，或由 ComfyUI 导出 API 图" })); };
            let mut summary = describe(graph);
            summary["sourceFormat"] = json!(kind);
            if args["includeApiJson"] == true { summary["apiJson"] = json!(graph.to_string()); }
            if args["checkInstalled"] == true { summary["installed"] = installed(&summary).await; }
            if args["includeNodeInputs"] == true {
                let base = env::var("DRAMLO_COMFY_URL").unwrap_or_else(|_| "http://127.0.0.1:8188".into());
                summary["nodeCatalog"] = node_catalog(&base,graph,&summary).await;
            }
            Ok(summary)
        }
        _ => Err(invalid("未知 ComfyUI 工作流工具")),
    }
}
