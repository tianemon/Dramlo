use super::*;
use std::{collections::HashSet, time::Duration};

pub(super) const METHODS: &[&str] = &[
    "studio_comfy_discover", "studio_comfy_instances", "studio_comfy_register",
    "studio_comfy_start", "studio_comfy_stop", "studio_comfy_remove", "studio_comfy_workflows", "studio_comfy_workflow_read", "studio_comfy_import",
];

#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Instance {
    id: String,
    name: String,
    root: Option<PathBuf>,
    port: u16,
}

impl Instance {
    fn address(&self) -> String { format!("http://127.0.0.1:{}", self.port) }
}

fn install(path: &Path) -> Option<PathBuf> {
    if path.join("main.py").is_file() && path.join("comfy").is_dir() {
        normalize_root(path).ok()
    } else { None }
}

fn python(root: &Path) -> Option<PathBuf> {
    let parent = root.parent().unwrap_or(root);
    [
        root.join(".venv/Scripts/python.exe"), root.join("venv/Scripts/python.exe"),
        parent.join("python_embeded/python.exe"), parent.join("python/python.exe"),
        root.join("python_embeded/python.exe"), root.join("python/python.exe"),
        root.join(".venv/bin/python"), root.join("venv/bin/python"),
    ].into_iter().find(|path| path.is_file())
}

fn configured_port(root: &Path) -> u16 {
    let parent = root.parent().unwrap_or(root);
    for directory in [root, parent] {
        let Ok(entries) = fs::read_dir(directory) else { continue; };
        for file in entries.flatten().take(100) {
            if !file.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd")) { continue; }
            if !file.metadata().is_ok_and(|meta| meta.is_file() && meta.len() <= 32768) { continue; }
            let Ok(text) = fs::read_to_string(file.path()) else { continue; };
            if !text.contains("main.py") { continue; }
            if let Some(port) = text.split_whitespace().collect::<Vec<_>>().windows(2)
                .find_map(|v| (v[0] == "--port").then(|| v[1].parse::<u16>().ok()).flatten()).filter(|port| *port > 0) {
                return port;
            }
        }
    }
    8188
}

fn scan_installations(input: &Value) -> Result<Vec<PathBuf>> {
    let mut pending = Vec::new();
    if let Some(items) = input.get("roots") {
        let items = items.as_array().filter(|items| items.len() <= 20).ok_or_else(|| invalid("扫描目录应是绝对路径数组"))?;
        for item in items {
            let path = item.as_str().map(PathBuf::from).filter(|path| path.is_absolute())
                .ok_or_else(|| invalid("扫描目录必须是绝对路径"))?;
            pending.push((path, 0));
        }
    } else {
        #[cfg(windows)]
        for drive in b'C'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", char::from(drive)));
            pending.push((root.join("Projects"), 0));
            pending.push((root.join("ComfyUI"), 0));
        }
        for variable in ["USERPROFILE", "HOME"] {
            if let Some(home) = env::var_os(variable) {
                for dir in ["ComfyUI", "Documents/ComfyUI", "Downloads/ComfyUI"] {
                    pending.push((PathBuf::from(&home).join(dir), 0));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some((path, depth)) = pending.pop() {
        let Ok(path) = normalize_root(&path) else { continue; };
        if seen.len() >= 700 { break; }
        if !seen.insert(path.clone()) { continue; }
        if let Some(path) = install(&path) { result.push(path); continue; }
        if depth == 4 { continue; }
        let Ok(entries) = fs::read_dir(&path) else { continue; };
        for entry in entries.flatten().take(250) {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name.starts_with('.') || ["node_modules", "python", "python_embeded", "models", "output", "assets", "data", "custom_nodes", "venv"].contains(&name.as_str()) { continue; }
            if depth == 0 && path.file_name().is_some_and(|part| part.eq_ignore_ascii_case("Projects")) && !name.contains("comfy") { continue; }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) { pending.push((entry.path(), depth + 1)); }
        }
    }
    result.sort();
    Ok(result)
}

async fn status(base: &str) -> Option<Value> {
    comfy_mcp::get_at(base, "/system_stats").await.ok()
        .filter(|stats| stats["system"].is_object() && stats["devices"].is_array())
}

fn reported_root(stats: &Value) -> Option<PathBuf> {
    let main = Path::new(stats["system"]["argv"][0].as_str()?);
    if !main.is_absolute() || main.file_name()? != "main.py" { return None; }
    install(main.parent()?)
}

fn verify_instance_address(instance: &Instance, stats: &Value) -> Result<()> {
    if let (Some(expected), Some(actual)) = (&instance.root, reported_root(stats)) {
        if *expected != actual { return Err(invalid("此端口正在运行另一个 ComfyUI 安装，请核对实例路径和端口")); }
    }
    Ok(())
}

fn instances(app: &App) -> Result<Vec<Instance>> {
    let path = kv_path(app, "comfy-instances")?;
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("ComfyUI 实例登记格式异常")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(error.into()),
    }
}

fn required<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input[key].as_str().map(str::trim).filter(|text| !text.is_empty()).ok_or_else(|| invalid("缺少必要字段"))
}

fn location(input: &Value) -> Result<(Option<PathBuf>, u16)> {
    let root = if let Some(value) = input.get("root") {
        let root = value.as_str().ok_or_else(|| invalid("安装目录须为字符串"))?;
        let supplied = Path::new(root);
        if !supplied.is_absolute() { return Err(invalid("请提供 ComfyUI 安装绝对路径")); }
        Some(install(supplied).or_else(|| install(&supplied.join("ComfyUI")))
            .ok_or_else(|| invalid("该目录没有可识别的 ComfyUI 主程序"))?)
    } else { None };
    if root.is_none() && input.get("port").is_none() { return Err(invalid("请输入安装目录或端口")); }
    if input.get("port").is_some() && !input["port"].as_u64().is_some_and(|port| (1..=65535).contains(&port)) {
        return Err(invalid("端口无效"));
    }
    let port = input["port"].as_u64().and_then(|port| u16::try_from(port).ok()).filter(|port| *port > 0)
        .unwrap_or_else(|| root.as_deref().map(configured_port).unwrap_or(8188));
    Ok((root, port))
}

fn instance_id(root: Option<&Path>, port: u16) -> String {
    let key = root.map(|root| root.to_string_lossy()).unwrap_or_default();
    let hash = key.as_bytes().iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("comfy-{port}-{hash:016x}")
}

fn browsable_instance(saved: &[Instance], input: &Value) -> Result<Instance> {
    match (input.get("instanceId"), input.get("target")) {
        (Some(id), None) => saved.iter().find(|instance| id == instance.id.as_str()).cloned()
            .ok_or_else(|| invalid("找不到已保存实例，请重新发现或选择目标")),
        (None, Some(target)) => {
            let (root, port) = location(target)?;
            if let Some(existing) = saved.iter().find(|instance| instance.root == root && instance.port == port) {
                return Ok(existing.clone());
            }
            Ok(Instance { id: instance_id(root.as_deref(), port), name: format!("ComfyUI {port}"), root, port })
        }
        (Some(_), Some(_)) => Err(invalid("instanceId 和 target 只能传一个：浏览发现的实例请保留 target，浏览已保存实例请保留 instanceId")),
        (None, None) => Err(invalid("请传入 discover 返回的 target，或已保存实例的 instanceId")),
    }
}

fn discovery_ports(saved: &[Instance]) -> Vec<u16> {
    let mut ports = vec![8188, 8189, 8190, 8191];
    ports.extend(saved.iter().map(|instance| instance.port));
    ports.sort_unstable();
    ports.dedup();
    ports
}

async fn automatic_catalog_target(saved: &[Instance]) -> Result<Instance> {
    let candidates = futures_util::future::join_all(discovery_ports(saved).into_iter().map(|port| async move {
        let address = format!("http://127.0.0.1:{port}");
        let stats = status(&address).await?;
        let root = reported_root(&stats);
        Some(Instance { id: instance_id(root.as_deref(), port), name: format!("ComfyUI {port}"), root, port })
    })).await.into_iter().flatten().collect::<Vec<_>>();
    match candidates.as_slice() {
        [instance] => Ok(instance.clone()),
        [] => Err(invalid("未找到在线的本机 ComfyUI。可运行 studio_comfy_discover，并传入所选安装的 target 读取磁盘工作流")),
        _ => Err(invalid(&format!("发现多个在线 ComfyUI（端口 {}），请传入所选端口的 target 或已保存实例的 instanceId",
            candidates.iter().map(|instance| instance.port.to_string()).collect::<Vec<_>>().join("、")))),
    }
}

fn register(app: &App, input: &Value) -> Result<Value> {
    if input["confirmed"] != true { return Err(invalid("请确认要保存的 ComfyUI 实例")); }
    let (root, port) = location(input)?;
    let name = input["name"].as_str().map(str::trim).filter(|name| !name.is_empty() && name.chars().count() <= 100)
        .map(str::to_owned).unwrap_or_else(|| format!("ComfyUI {port}"));
    let _write = app.writes.lock().map_err(|_| invalid("配置写锁不可用"))?;
    let mut entries = instances(app)?;
    if let Some(existing) = entries.iter().find(|entry| entry.root == root && entry.port == port) {
        return Ok(json!({ "instance": existing, "reused": true }));
    }
    let result = if let Some(existing) = entries.iter_mut().find(|instance| instance.port == port) {
        if existing.root.is_some() && root.is_some() {
            return Err(invalid("此端口已登记给另一个实例，请为它选择独立端口"));
        }
        if let Some(root) = root { existing.root = Some(root); }
        if input.get("name").is_some() { existing.name = name; }
        json!({ "instance": existing, "reused": true })
    } else {
        if entries.len() >= 32 { return Err(invalid("登记实例数量已达上限")); }
        let id = instance_id(root.as_deref(), port);
        let instance = Instance { id, name, root, port };
        let result = json!({ "instance": instance, "reused": false });
        entries.push(instance);
        result
    };
    save_instances(app, &entries)?;
    Ok(result)
}

fn save_instances(app: &App, entries: &[Instance]) -> Result<()> {
    atomic_save(&kv_path(app, "comfy-instances")?, &serde_json::to_vec(entries).map_err(|_| invalid("无法保存实例"))?)
}

fn workflow_roots(instance: &Instance) -> Vec<PathBuf> {
    let Some(root) = &instance.root else { return vec![]; };
    let mut candidates = Vec::new();
    if let Ok(users) = fs::read_dir(root.join("user")) {
        for user in users.flatten().take(32) {
            if user.file_type().is_ok_and(|kind| kind.is_dir()) { candidates.push(user.path().join("workflows")); }
        }
    }
    candidates.extend([root.join("workflows"), root.join("workflow-api"), root.join("api-workflows")]);
    let mut roots: Vec<_> = candidates.into_iter().filter_map(|dir| normalize_root(&dir).ok())
        .filter(|dir| dir.is_dir() && dir.starts_with(root)).collect();
    roots.sort();
    roots.dedup();
    roots
}

fn item_origin(instance: &Instance, item: &Value, roots: &[PathBuf]) -> Value {
    let root = item["folder"].as_u64().and_then(|folder| roots.get(folder as usize));
    json!({"instanceId":instance.id,"source":item["source"],"root":root,"relativePath":item["relativePath"]})
}

fn same_origin(left: &Value, right: &Value) -> bool {
    left.is_object() && left["source"] == right["source"] && left["root"] == right["root"]
        && left["relativePath"].as_str().zip(right["relativePath"].as_str())
            .is_some_and(|(left,right)| left.replace('\\', "/") == right.replace('\\', "/"))
}

fn endpoint_matches(profile: &Value, address: &str) -> bool {
    profile["adapter"]["type"] == "comfyui-prompt" &&
        profile["baseUrl"].as_str().is_some_and(|url| url.trim_end_matches('/') == address)
}

fn workflow_path(relative: &str) -> Result<String> {
    if relative.len() > 2048 || !relative.ends_with(".json") || relative.contains('\\') ||
        relative.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(invalid("工作流相对路径无效"));
    }
    Ok(format!("/userdata/{}", video_queue::escape(&format!("workflows/{relative}"))))
}

async fn merge_online_workflows(
    address: &str, roots: &[PathBuf], items: &mut Vec<Value>, online: Vec<Value>,
) {
    let mut unmatched = Vec::new();
    let mut checks = 0;
    let started = std::time::Instant::now();
    for mut item in online {
        let relative = item["relativePath"].as_str().unwrap_or_default();
        let matching_path = |local: &Value| local["relativePath"] == relative
            && local["source"] == "disk" && local["alternateLocation"].is_null();
        // A shared name may refer to different workflows. Bound the extra reads and merge only verified copies.
        if checks < 24 && started.elapsed() < Duration::from_secs(3)
            && items.iter().any(matching_path) {
            checks += 1;
            if let Ok(route) = workflow_path(relative) {
                if let Ok(Ok(remote)) = tokio::time::timeout(
                    Duration::from_millis(750), comfy_mcp::get_at(address,&route)).await {
                    let matched = items.iter_mut().filter(|local| matching_path(local)).find(|local| {
                        local["folder"].as_u64().and_then(|n| roots.get(n as usize))
                            .is_some_and(|root| comfy_mcp::disk(root,Path::new(relative)).is_ok_and(|file|file == remote))
                    });
                    if let Some(local) = matched {
                        item["format"] = local["format"].clone();
                        local["alternateLocation"] = item;
                        continue;
                    }
                }
            }
        }
        unmatched.push(item);
    }
    items.extend(unmatched);
}

async fn list_workflows(app: &App, instance: &Instance) -> Result<Value> {
    let roots = workflow_roots(instance);
    let address = instance.address();
    let mut items = Vec::new();
    let mut problems = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let mut remaining = 1000;
        if let Err(error) = comfy_mcp::scan(root, Path::new(""), index, &mut remaining, 0, &mut items) {
            problems.push(error.1);
        }
    }
    for item in &mut items {
        if let Some(relative) = item["relativePath"].as_str() {
            item["relativePath"] = json!(relative.replace('\\', "/"));
        }
    }
    match status(&address).await {
        Some(stats) => match verify_instance_address(instance,&stats) {
            Ok(()) => match comfy_mcp::get_at(&address, "/userdata?dir=workflows&recurse=true&split=false&full_info=true").await {
                Ok(value) => {
                    let mut seen = HashSet::new();
                    let online = value.as_array().into_iter().flatten().take(500).filter_map(|item| {
                        let path = item.as_str().or_else(|| item["path"].as_str())?;
                        let relative = path.strip_prefix("workflows/").unwrap_or(path);
                        if workflow_path(relative).is_err() || !seen.insert(relative) { return None; }
                        Some(json!({"source":"server","relativePath":relative,"name":relative.rsplit('/').next()}))
                    }).collect();
                    merge_online_workflows(&address,&roots,&mut items,online).await;
                }
                Err(error) => problems.push(error.1),
            },
            Err(error) => problems.push(error.1),
        },
        None if items.is_empty() => problems.push("该端口没有可验证的在线 ComfyUI，可以选择安装目录浏览磁盘工作流".into()),
        None => {}
    }

    let mut unlinked = Vec::new();
    match settings(app) {
        Ok((_, config)) => {
            let mut linked = Vec::new();
            for (kind, group) in [("image", "images"), ("video", "videos")] {
                for profile in config[group].as_array().into_iter().flatten().filter(|p| endpoint_matches(p,&address)) {
                    let entry = json!({"profileId":profile["id"],"name":profile["name"],"kind":kind});
                    let origin = &profile["adapter"]["workflowOrigin"];
                    if origin.is_object() { linked.push((entry,origin)); }
                    else { unlinked.push(entry); }
                }
            }
            for item in &mut items {
                let origin = item_origin(instance,item,&roots);
                let alternate_origin = item["alternateLocation"].is_object()
                    .then(|| item_origin(instance,&item["alternateLocation"],&roots));
                let mut primary = false;
                let mut profiles = Vec::new();
                for (entry, saved) in &linked {
                    if same_origin(saved,&origin) {
                        primary = true;
                        profiles.push(entry);
                    } else if alternate_origin.as_ref().is_some_and(|alt| same_origin(saved,alt)) {
                        profiles.push(entry);
                    }
                }
                if !primary && !profiles.is_empty() {
                    let mut preferred = std::mem::take(&mut item["alternateLocation"]);
                    if let Some(object) = item.as_object_mut() { object.remove("alternateLocation"); }
                    preferred["alternateLocation"] = std::mem::take(item);
                    *item = preferred;
                }
                item["configurationStatus"] = json!(if profiles.is_empty() { "available" } else { "configured" });
                item["profiles"] = json!(profiles);
            }
        }
        Err(error) => {
            problems.push(format!("无法核对已配置状态：{}",error.1));
            for item in &mut items { item["configurationStatus"] = json!("unknown"); }
        }
    }
    items.sort_by(|a, b| a["relativePath"].as_str().cmp(&b["relativePath"].as_str()));
    Ok(json!({ "instanceId": instance.id, "address":address, "roots": roots, "items": items,
        "unlinkedProfiles":unlinked, "problems": problems }))
}

pub(super) async fn convert_ui(address: &str, raw: &Value) -> Result<Value> {
    let address = address.trim_end_matches('/');
    if !address.strip_prefix("http://127.0.0.1:").and_then(|port| port.parse::<u16>().ok()).is_some_and(|port| port > 0) {
        return Err(invalid("只支持本机回环 ComfyUI 的前端自动转换"));
    }
    #[cfg(windows)]
    {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use std::os::windows::process::CommandExt;
        let temp = tempfile::tempdir()?;
        fs::write(temp.path().join("graph.json"), raw.to_string())?;
        let code = include_str!("comfy-export.ps1").encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        let powershell = PathBuf::from(env::var_os("SystemRoot").ok_or_else(|| invalid("找不到 Windows PowerShell"))?)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = std::process::Command::new(powershell);
        command.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &STANDARD.encode(code)])
            .env_remove("PSModulePath").env("DRAMLO_COMFY_CONVERT_DIR", temp.path())
            .env("DRAMLO_COMFY_CONVERT_URL", address).creation_flags(0x08000000);
        let mut child = tokio::process::Command::from(command);
        child.kill_on_drop(true);
        let output = tokio::time::timeout(Duration::from_secs(55), child.output()).await
            .map_err(|_| invalid("ComfyUI 自动转换超时"))??;
        if !output.status.success() {
            return Err(invalid("自动转换失败，请检查该实例已启动、Chrome 或 Edge 可用及工作流所需节点已安装"));
        }
        let path = temp.path().join("api.json");
        if fs::metadata(&path)?.len() > 2 * 1024 * 1024 { return Err(invalid("转换后的图超出大小限制")); }
        serde_json::from_slice(&fs::read(path)?).map_err(|_| invalid("ComfyUI 自动转换没有返回有效 API 图"))
    }
    #[cfg(not(windows))]
    { let _ = (address, raw); Err(invalid("自动转换界面工作流目前需要 Windows")) }
}

pub(super) fn settings(app: &App) -> Result<(PathBuf, Value)> {
    let path = kv_path(app, "settings")?;
    let data = match fs::read(&path) {
        Ok(data) => {
            let row: Value = serde_json::from_slice(&data).map_err(|_| invalid("现有设置格式无效"))?;
            serde_json::from_str(row["value"].as_str().ok_or_else(|| invalid("现有设置格式无效"))?)
                .map_err(|_| invalid("现有设置格式无效"))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.into()),
    };
    if !data.is_object() { return Err(invalid("现有设置格式无效")); }
    Ok((path, data))
}

async fn read_workflow(instance: &Instance, input: &Value) -> Result<(Value, Value, bool)> {
    let relative = required(input, "relativePath")?;
    if relative.len() > 2048 { return Err(invalid("工作流路径过长")); }
    let source = required(input, "source")?;
    let folder = input["folder"].as_u64().unwrap_or(0) as usize;
    let mut source_root = None;
    let raw = match source {
        "disk" => {
            let roots = workflow_roots(instance);
            let root = roots.get(folder).ok_or_else(|| invalid("找不到对应工作流目录"))?;
            source_root = Some(root.clone());
            comfy_mcp::disk(root, Path::new(relative))?
        }
        "server" => {
            let stats = status(&instance.address()).await.ok_or_else(|| invalid("该端口没有在线的 ComfyUI 实例"))?;
            verify_instance_address(instance, &stats)?;
            comfy_mcp::get_at(&instance.address(), &workflow_path(relative)?).await?
        }
        _ => return Err(invalid("工作流来源无效")),
    };
    let (format, api) = comfy_mcp::format(&raw);
    let api = match api {
        Some(graph) => graph.clone(),
        None if format == "ui" => {
            let stats = status(&instance.address()).await.ok_or_else(|| invalid("请先启动此实例以自动转换界面工作流"))?;
            verify_instance_address(instance, &stats)?;
            convert_ui(&instance.address(), &raw).await?
        }
        _ => return Err(invalid("无法识别工作流格式")),
    };
    let (_, graph) = comfy_mcp::format(&api);
    let graph = graph.ok_or_else(|| invalid("未取得可执行的 ComfyUI API 图"))?;
    let origin = json!({ "instanceId": instance.id, "source": source, "root": source_root, "relativePath": relative });
    Ok((graph.clone(), origin, format == "ui"))
}

async fn import(app: &App, instance: &Instance, input: &Value) -> Result<Value> {
    let (graph, origin, converted) = read_workflow(instance, input).await?;
    if instance.root.is_some() && input["source"] == "disk" && !converted {
        if let Some(stats) = status(&instance.address()).await {
            verify_instance_address(instance, &stats)?;
        }
    }
    let source_format = if converted { "ui" } else { "api" };
    let graph = &graph;

    let summary = comfy_mcp::describe(graph);
    let kind = match input["kind"].as_str().or_else(|| summary["output"].as_str()) {
        Some(kind @ ("image" | "video")) => kind,
        _ => return Err(invalid("无法识别工作流输出类型，请指定图片或视频")),
    };
    let fingerprint = format!("{:016x}",video_queue::stamp(graph));
    if input["dryRun"] == true {
        let candidate_slots = &summary["candidateSlots"];
        let roles = ["prompt","length","width","height","aspect_ratio","image","imageEnd"];
        let counts: serde_json::Map<String,Value> = roles.into_iter().map(|role| (role.into(),json!(
            candidate_slots.as_array().into_iter().flatten().filter(|slot| slot["role"] == role).count()
        ))).collect();
        return Ok(json!({"dryRun":true,"applied":false,"kind":kind,"sourceFormat":source_format,"origin":origin,
            "workflowFingerprint":fingerprint,"candidateSlots":candidate_slots,"counts":counts,
            "nextAction":"核对参考图槽的用途、是否必填和中性补位图，然后带 workflowFingerprint、referenceConfig 或 outputConfig 调用导入"}));
    }
    if input.get("workflowFingerprint").is_some_and(|value| value != &fingerprint) {
        return Err(Failure::new(StatusCode::CONFLICT,"来源工作流已变更，请重新预览导入".into()));
    }
    let (group, active_key, mirror_key) = if kind == "video" { ("videos", "activeVideoId", "video") }
        else { ("images", "activeImageId", "image") };
    let _write = app.writes.lock().map_err(|_| invalid("设置写锁不可用"))?;
    let (path, mut config) = settings(app)?;
    if config[group].is_null() { config[group] = json!([]); }
    let profiles = config[group].as_array_mut().ok_or_else(|| invalid("接入配置格式无效"))?;
    let address = instance.address();
    let existing = profiles.iter().position(|profile|
        endpoint_matches(profile,&address) && same_origin(&profile["adapter"]["workflowOrigin"],&origin));
    let name = input["name"].as_str().map(str::trim).filter(|value| !value.is_empty() && value.chars().count() <= 120);
    let (mut profile, changed) = if let Some(index) = existing {
        let profile = profiles[index].clone();
        let stored = profile["adapter"]["workflowTemplate"].as_str()
            .and_then(|template| serde_json::from_str::<Value>(template).ok());
        (profile, stored.as_ref() != Some(graph))
    } else {
        let base_id = format!("{}-{}", instance.id, profiles.len().saturating_add(1));
        let id = (0..1000).map(|n| if n == 0 { base_id.clone() } else { format!("{base_id}-{n}") })
            .find(|id| !profiles.iter().any(|profile| profile["id"] == *id)).ok_or_else(|| invalid("无法分配工作流 ID"))?;
        let filename = Path::new(origin["relativePath"].as_str().unwrap_or_default()).file_stem()
            .and_then(|name| name.to_str()).unwrap_or("ComfyUI 工作流");
        let mut profile = json!({ "id": id, "name": filename, "provider": "custom", "apiKey": "",
            "baseUrl": instance.address(), "adapter": { "type": "comfyui-prompt", "authPrefix": "", "workflowOrigin": origin } });
        if kind == "video" {
            profile["workflowId"] = json!("comfyui-local");
            profile["stillMode"] = json!("keyframe");
        } else {
            profile["model"] = json!("comfyui");
            profile["models"] = json!(["comfyui"]);
        }
        (profile, true)
    };
    if let Some(name) = name { profile["name"] = json!(name); }
    if changed {
        profile["adapter"]["workflowTemplate"] = json!(graph.to_string());
        let adapter = profile["adapter"].as_object_mut().ok_or_else(|| invalid("适配器配置无效"))?;
        adapter.remove("inputMap");
        let bindings = if kind == "video" { video_queue::inferred_bindings(graph) }
            else { image_queue::inferred_mapping(graph) };
        if !bindings.is_empty() { adapter.insert("inputMap".into(), json!({ "bindings": bindings })); }
    }
    comfy_config::update(&mut profile,input,kind)?;
    let video_diagnosis = (kind == "video").then(|| video_queue::describe_workflow(&profile));
    let missing: Vec<&str> = if kind == "video" {
        let bindings = profile["adapter"]["inputMap"]["bindings"].as_array().map_or(&[][..], Vec::as_slice);
        if let Some(Ok(diagnosis)) = &video_diagnosis {
            ["prompt","length"].into_iter().filter(|role| diagnosis["missingMapping"].as_array()
                .is_some_and(|items| items.iter().any(|item| item == *role))).collect()
        } else {
            ["prompt","length"].into_iter().filter(|role| !bindings.iter().any(|binding| binding["role"] == *role)).collect()
        }
    } else {
        let has_prompt = profile["adapter"]["inputMap"]["bindings"].as_array().into_iter().flatten()
            .any(|binding| binding["role"] == "prompt");
        if has_prompt || image_queue::inferred_mapping(graph).iter().any(|binding| binding["role"] == "prompt") { vec![] }
        else { vec!["prompt"] }
    };
    let mapping_issue = if kind == "image" { image_queue::mapping_issue(&profile) }
        else { video_diagnosis.and_then(Result::err).map(|error| error.1) };
    let assistance = json!({"referenceConfig":profile["adapter"]["referenceConfig"],"outputConfig":profile["adapter"]["outputConfig"]});
    let id = profile["id"].clone();
    let profile_name = profile["name"].clone();
    if let Some(index) = existing { profiles[index] = profile.clone(); } else { profiles.push(profile.clone()); }
    let selected = missing.is_empty() && mapping_issue.is_none() && input["select"] == true;
    if selected || config[active_key] == id {
        config[active_key] = id.clone();
        config[mirror_key] = profile;
    }
    atomic_save(&path, json!({ "key": "settings", "value": config.to_string() }).to_string().as_bytes())?;
    Ok(json!({ "profileId": id, "profileName":profile_name, "kind": kind, "reused": existing.is_some(), "updated": existing.is_some() && changed,
        "sourceFormat": source_format, "workflowFingerprint":fingerprint, "missingMapping": missing,
        "mappingIssue":mapping_issue, "converted": converted, "selected": selected, "config":assistance }))
}

fn owner_path(app: &App, instance: &Instance) -> Result<PathBuf> {
    kv_path(app, &format!("comfy-owner:{}", instance.id))
}

fn owner(app: &App, instance: &Instance) -> Result<Option<managed_process::Identity>> {
    match fs::read(owner_path(app, instance)?) {
        Ok(data) => {
            let identity = serde_json::from_slice(&data).map_err(|_| invalid("ComfyUI 进程记录无效"))?;
            Ok(managed_process::alive(identity)?.then_some(identity))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn clear_owner(app: &App, instance: &Instance) -> Result<()> {
    match fs::remove_file(owner_path(app, instance)?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn lifecycle(app: &App, instance: &Instance, action: &str, input: &Value) -> Result<Value> {
    if input["confirmed"] != true { return Err(invalid("请确认要启停的 ComfyUI 实例")); }
    let mut children = app.comfy_children.lock().await;
    if children.get_mut(&instance.id).is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some())) {
        children.remove(&instance.id);
    }
    let identity = owner(app, instance)?;
    let address = instance.address();
    if action == "studio_comfy_start" {
        if let Some(stats) = status(&address).await {
            verify_instance_address(instance, &stats)?;
            return Ok(json!({ "online": true, "reused": true, "managed": identity.is_some() || children.contains_key(&instance.id) }));
        }
        if identity.is_some() || children.contains_key(&instance.id) { return Ok(json!({ "online": false, "starting": true, "managed": true })); }
        if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, instance.port)).await.is_ok() {
            return Err(invalid("端口已被其他程序占用"));
        }
        let root = instance.root.as_deref().and_then(install).ok_or_else(|| invalid("此实例没有可启动的本机路径"))?;
        let python = python(&root).ok_or_else(|| invalid("未发现实例自己的 Python 或虚拟环境"))?;
        let logs = state_directory(&app.root)?;
        fs::create_dir_all(&logs)?;
        let stdout = fs::File::create(logs.join(format!("{}.out.log", instance.id)))?;
        let stderr = fs::File::create(logs.join(format!("{}.err.log", instance.id)))?;
        let mut command = tokio::process::Command::new(&python);
        command.current_dir(&root).arg("-s").arg(root.join("main.py")).args(["--listen", "127.0.0.1", "--port", &instance.port.to_string()])
            .stdin(std::process::Stdio::null()).stdout(stdout).stderr(stderr);
        #[cfg(windows)] { command.creation_flags(0x08000000); }
        let mut child = command.spawn()?;
        let record = (|| -> Result<()> {
            let pid = child.id().ok_or_else(|| invalid("ComfyUI 进程未启动"))?;
            if let Some(identity) = managed_process::identify(pid)? {
                let bytes = serde_json::to_vec(&identity).map_err(|_| invalid("无法保存 ComfyUI 进程身份"))?;
                atomic_save(&owner_path(app, instance)?, &bytes)?;
            }
            Ok(())
        })();
        if let Err(error) = record {
            let _ = child.kill().await;
            return Err(error);
        }
        children.insert(instance.id.clone(), child);
        return Ok(json!({ "online": false, "starting": true, "managed": true }));
    }
    if identity.is_none() && !children.contains_key(&instance.id) {
        return Err(invalid("此实例未由 Dramlo 启动，保留外部实例"));
    }
    let stats = status(&address).await.ok_or_else(|| invalid("无法核对 ComfyUI 在线状态"))?;
    verify_instance_address(instance, &stats)?;
    let queue = comfy_mcp::get_at(&address, "/queue").await?;
    if !["queue_running", "queue_pending"].iter().all(|key| queue[*key].as_array().is_some_and(Vec::is_empty)) {
        return Err(invalid("ComfyUI 队列有任务或状态无法核对"));
    }
    if let Some(child) = children.get_mut(&instance.id) {
        child.kill().await?;
    } else if let Some(identity) = identity {
        let stopped = tokio::task::spawn_blocking(move || managed_process::stop(identity)).await
            .map_err(|_| invalid("无法等待 ComfyUI 停止"))??;
        if !stopped { return Err(invalid("ComfyUI 进程已变更，请刷新后重试")); }
    }
    children.remove(&instance.id);
    clear_owner(app, instance)?;
    Ok(json!({ "stopped": true }))
}

pub(super) async fn call(app: &App, name: &str, input: &Value) -> Result<Value> {
    let saved = instances(app)?;
    match name {
        "studio_comfy_discover" => {
            let mut roots = scan_installations(input)?;
            let mut running = Vec::new();
            let mut ports = discovery_ports(&saved);
            if let Some(value) = input.get("ports") {
                ports = value.as_array().filter(|items| items.len() <= 20).ok_or_else(|| invalid("端口列表无效"))?
                    .iter().map(|port| port.as_u64().and_then(|port| u16::try_from(port).ok()).filter(|port| *port != 0)
                        .ok_or_else(|| invalid("端口无效"))).collect::<Result<Vec<_>>>()?;
            }
            ports.sort_unstable();
            ports.dedup();
            for port in ports {
                let address = format!("http://127.0.0.1:{port}");
                if let Some(stats) = status(&address).await {
                    let root = reported_root(&stats);
                    if input.get("roots").is_none() {
                        if let Some(path) = &root {
                            if !roots.contains(path) { roots.push(path.clone()); }
                        }
                    }
                    running.push((port, address, root));
                }
            }
            let installations = roots.iter().map(|root| {
                let match_port = running.iter().find(|(_, _, path)| path.as_ref() == Some(root)).map(|row| row.0);
                let port = match_port.unwrap_or_else(|| configured_port(root));
                json!({ "root": root, "suggestedPort": port, "target":{"root":root,"port":port},
                    "online": match_port.is_some(), "canStart": python(root).is_some() })
            }).collect::<Vec<_>>();
            let running = running.into_iter().map(|(port, address, root)| json!({ "port": port, "address": address, "root": root, "target":{"port":port} })).collect::<Vec<_>>();
            Ok(json!({ "installations": installations, "running": running, "registered": saved }))
        }
        "studio_comfy_instances" => {
            let mut rows = Vec::new();
            for instance in saved {
                let mut entry = json!(instance);
                let stats = status(&instance.address()).await;
                let conflict = stats.as_ref().is_some_and(|stats| verify_instance_address(&instance, stats).is_err());
                entry["online"] = json!(stats.is_some() && !conflict);
                entry["portConflict"] = json!(conflict);
                let mut children = app.comfy_children.lock().await;
                if children.get_mut(&instance.id).is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some())) {
                    children.remove(&instance.id);
                }
                entry["managed"] = json!(owner(app, &instance)?.is_some() || children.contains_key(&instance.id));
                rows.push(entry);
            }
            Ok(json!({ "instances": rows }))
        }
        "studio_comfy_register" => register(app, input),
        "studio_comfy_workflows" | "studio_comfy_workflow_read" | "studio_comfy_import" => {
            let instance = if name == "studio_comfy_workflows" && input.get("target").is_none() && input.get("instanceId").is_none() {
                automatic_catalog_target(&saved).await?
            } else {
                browsable_instance(&saved, input)?
            };
            match name {
                "studio_comfy_workflows" => {
                    let mut result = list_workflows(app,&instance).await?;
                    let locator = if input.get("instanceId").is_none() {
                        result.as_object_mut().unwrap().remove("instanceId");
                        let mut target = json!({"port":instance.port});
                        if let Some(root) = &instance.root { target["root"] = json!(root); }
                        result["target"] = target.clone();
                        json!({"target":target})
                    } else {
                        json!({"instanceId":instance.id})
                    };
                    if let Some(items) = result["items"].as_array_mut() {
                        for item in items {
                            let mut location = locator.clone();
                            location["source"] = item["source"].clone();
                            location["relativePath"] = item["relativePath"].clone();
                            if let Some(folder) = item.get("folder") { location["folder"] = folder.clone(); }
                            item["readInput"] = location;
                        }
                    }
                    Ok(result)
                }
                "studio_comfy_workflow_read" => {
                    let (graph, origin, converted) = read_workflow(&instance, input).await?;
                    let mut summary = comfy_mcp::describe(&graph);
                    summary["origin"] = origin;
                    summary["converted"] = json!(converted);
                    if input["includeApiJson"] == true { summary["apiJson"] = json!(graph.to_string()); }
                    if input["includeNodeInputs"] == true {
                        summary["nodeCatalog"] = comfy_mcp::node_catalog(&instance.address(),&graph,&summary).await;
                    }
                    Ok(summary)
                }
                _ => import(app, &instance, input).await,
            }
        }
        _ => {
            let id = required(input, "instanceId")?;
            let instance = saved.iter().find(|item| item.id == id).ok_or_else(|| invalid("托管操作需要已保存的实例"))?;
            match name {
                "studio_comfy_start" | "studio_comfy_stop" => lifecycle(app, instance, name, input).await,
                "studio_comfy_remove" => {
                    if input["confirmed"] != true { return Err(invalid("请确认移除实例登记")); }
                    let mut children = app.comfy_children.lock().await;
                    if owner(app, instance)?.is_some() || children.get_mut(id).is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_none())) {
                        return Err(invalid("请先停止工作台启动的此实例"));
                    }
                    children.remove(id);
                    clear_owner(app, instance)?;
                    let _write = app.writes.lock().map_err(|_| invalid("配置写锁不可用"))?;
                    let mut entries = instances(app)?;
                    entries.retain(|item| item.id != id);
                    save_instances(app, &entries)?;
                    Ok(json!({ "removed": true }))
                }
                _ => Err(invalid("未知 ComfyUI 实例操作")),
            }
        }
    }
}

#[derive(Deserialize)]
pub(super) struct RequestBody { name: String, input: Value }

pub(super) async fn browser(State(app): State<App>, Json(request): Json<RequestBody>) -> Result<Json<Value>> {
    if !METHODS.contains(&request.name.as_str()) { return Err(invalid("未知 ComfyUI 实例操作")); }
    Ok(Json(call(&app, &request.name, &request.input).await?))
}
