mod managed_process;
mod managed_comfy;
mod launcher_token;
mod studio_mcp;
mod stdio_mcp;
mod comfy_mcp;
mod comfy_config;
mod project_authoring;
mod canvas_write;
mod capture_plan;
mod canvas_extensions;
mod extension_stdio;
mod extension_package;
mod video_queue;
mod image_queue;
mod generation_review;
mod openai_images;
mod final_export;
mod execution_plan;
mod workspace;
mod workspace_backup;
mod remote_video;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    env, fs, io,
    net::{Ipv4Addr, SocketAddr},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use tower::ServiceExt;
use tower_http::{services::{ServeDir, ServeFile}, set_header::SetResponseHeaderLayer};

#[derive(Clone)]
struct App {
    root: PathBuf,
    address: SocketAddr,
    writes: Arc<Mutex<()>>,
    active_project: Arc<Mutex<Option<String>>>,
    mcp_launcher_token: bool,
    shutdown: tokio::sync::watch::Sender<bool>,
    project_revision: tokio::sync::watch::Sender<Option<Value>>,
    video_active: Arc<Mutex<Option<(String, String)>>>,
    plan_active: Arc<Mutex<Option<(String, String)>>>,
    comfy_children: Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::process::Child>>>,
}

#[derive(Debug)]
struct Failure(StatusCode, String, Option<(Option<u64>, Option<u64>)>, Vec<Value>);
type Result<T> = std::result::Result<T, Failure>;

impl Failure {
    fn new(status: StatusCode, message: String) -> Self {
        Self(status, message, None, vec![])
    }

    fn revision_conflict(current: Option<u64>, expected: Option<u64>) -> Self {
        Self(StatusCode::CONFLICT, "项目已更新，请重新读取".into(), Some((current, expected)), vec![])
    }

    fn with_details(mut self, data: &Value) -> Self {
        if self.0 == StatusCode::CONFLICT && data.get("currentUpdatedAt").is_some() {
            self.2 = Some((data["currentUpdatedAt"].as_u64(), data["expectedUpdatedAt"].as_u64()));
        }
        if let Some(issues) = data["issues"].as_array() { self.3 = issues.clone(); }
        self
    }

    fn with_issues(mut self, issues: Vec<Value>) -> Self {
        self.3 = issues;
        self
    }

    fn payload(&self) -> Value {
        let code = match self.0 {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY | StatusCode::PAYLOAD_TOO_LARGE => "invalid",
            StatusCode::NOT_FOUND => "not_found",
            StatusCode::CONFLICT => "conflict",
            StatusCode::LOCKED | StatusCode::TOO_MANY_REQUESTS => "busy",
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "forbidden",
            StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => "timeout",
            _ => "internal",
        };
        let mut data = json!({ "error": self.1, "code": code });
        if let Some((current, expected)) = self.2 {
            data["currentUpdatedAt"] = json!(current);
            data["expectedUpdatedAt"] = json!(expected);
        }
        if !self.3.is_empty() { data["issues"] = json!(self.3); }
        data
    }

    fn display_message(&self) -> String {
        match self.2 {
            Some((Some(current),Some(expected))) => format!("{}（服务端当前 {current}，请求版本 {expected}）",self.1),
            _ => self.1.clone(),
        }
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.0, Json(self.payload())).into_response()
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::NotFound => StatusCode::NOT_FOUND,
            io::ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(code, error.to_string())
    }
}

fn invalid(reason: &str) -> Failure {
    Failure::new(StatusCode::BAD_REQUEST, reason.into())
}

/// 解析 ffmpeg 位置：环境变量优先，其次是发行包自带的 ffmpeg/。
///
/// 自带一份是为了开箱可用——让用户自己去装并配 PATH，会让截尾帧、
/// 导出和合成都变成"看环境"的功能。
pub(crate) fn ffmpeg_binary() -> Result<PathBuf> {
    if let Some(path) = env::var_os("DRAMLO_FFMPEG_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
    {
        return Ok(path);
    }
    let exe = env::current_exe()?;
    let local = exe
        .parent()
        .ok_or_else(|| invalid("找不到发行包目录"))?
        .join("ffmpeg")
        .join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    if local.is_file() {
        return Ok(local);
    }
    Err(invalid(
        "未找到 ffmpeg：把它放进发行包的 ffmpeg/ 目录，或设置 DRAMLO_FFMPEG_BIN",
    ))
}

fn normalize_root(root: &Path) -> io::Result<PathBuf> {
    let resolved = fs::canonicalize(root)?;
    #[cfg(windows)]
    if let Some(path) = resolved.to_str().and_then(|path| path.strip_prefix(r"\\?\")) {
        return Ok(PathBuf::from(path));
    }
    Ok(resolved)
}

fn scoped_path(root: &Path, requested: &Path) -> Result<PathBuf> {
    let relative = if requested.is_absolute() {
        requested.strip_prefix(root).map_err(|_| invalid("文件不在 Dramlo 工作区"))?
    } else {
        requested
    };
    let hidden_root = relative.components().next().is_some_and(|part| {
        let name = part.as_os_str().to_string_lossy();
        name.starts_with('.') && name != ".dramlo-import"
    });
    if relative.components().any(|part| !matches!(part, Component::Normal(_)))
        || hidden_root || relative == Path::new("app.db") {
        return Err(invalid("该工作区路径不可访问"));
    }
    let target = root.join(relative);
    let mut path = root.to_owned();
    for part in relative.components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                if !normalize_root(&path)?.starts_with(root) {
                    return Err(invalid("工作区链接指向了外部位置"));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(target)
}

fn validate_project_revision(input: &Value, current: u64, allow_auto: bool) -> Result<()> {
    let expected = &input["expectedUpdatedAt"];
    if expected.as_u64() == Some(current) || (allow_auto && expected == "auto") {
        return Ok(());
    }
    if input.get("expectedUpdatedAt").is_some() && expected.as_u64().is_none() {
        return Err(invalid(if allow_auto {
            "expectedUpdatedAt 须为项目修订号或 auto"
        } else {
            "此操作须传入核对过的数字项目修订号"
        }));
    }
    Err(Failure::revision_conflict(Some(current), expected.as_u64()))
}

fn notify_project_revision(app: &App, value: Value) {
    app.project_revision.send_if_modified(|last| {
        if last.as_ref() == Some(&value) { return false; }
        *last = Some(value);
        true
    });
}

fn watch_project_revisions(app: App) {
    let mut stop = app.shutdown.subscribe();
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(std::time::Duration::from_millis(500));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.changed() => break,
                _ = ticks.tick() => {
                    let Ok(_guard) = app.writes.try_lock() else { continue };
                    let Ok(project) = studio_mcp::active_project(&app) else { continue };
                    let value = project.map_or(Value::Null, |(_, project)|
                        json!({"projectId": project["id"],"updatedAt": project["updatedAt"]}));
                    notify_project_revision(&app, value);
                }
            }
        }
    });
}

async fn project_events(State(app): State<App>) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::convert::Infallible;

    let stop = app.shutdown.subscribe();
    let updates = app.project_revision.subscribe();
    let events = futures_util::stream::unfold((stop, updates, true), |(mut stop, mut updates, initial)| async move {
        if *stop.borrow() { return None; }
        if !initial || updates.borrow().is_none() {
            tokio::select! {
                _ = stop.changed() => return None,
                changed = updates.changed() => changed.ok()?,
            }
        }
        let snapshot = updates.borrow_and_update().clone()?;
        let event = Event::default().event("project").data(snapshot.to_string());
        Some((Ok::<_, Infallible>(event), (stop, updates, false)))
    });
    Sse::new(events).keep_alive(KeepAlive::default())
}


fn atomic_save(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write;
    let directory = path.parent().ok_or_else(|| invalid("缺少保存目录"))?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(contents)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

async fn guard(State(app): State<App>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let address = app.address.to_string();
    let origin = format!("http://{address}");
    let is_api = request.uri().path().starts_with("/api/");
    let is_app_shell = matches!(request.uri().path(), "/" | "/index.html");
    let is_media = request.uri().path() == "/api/dramlo/file" && request.method().is_safe();
    let is_backup_download = request.uri().path() == "/api/dramlo/workspace-backup"
        && request.method() == "GET" && headers.get("sec-fetch-site").is_some_and(|value| value == "same-origin");
    let is_project_events = request.uri().path() == "/api/dramlo/project-events"
        && request.method().is_safe() && headers.get("sec-fetch-site").is_some_and(|value| value == "same-origin");
    let is_mcp = matches!(request.uri().path(), "/api/dramlo/mcp" | "/api/dramlo/mcp-capabilities");
    // 本机直连（不带 Origin：CLI、MCP、外部 Agent）视为可信客户端，不再校验令牌。
    // 浏览器请求必须同源，且带 x-dramlo-ui 标记；跨站请求被 Origin 拦下。
    let local_client = !headers.contains_key(header::ORIGIN);
    let host = headers.get(header::HOST).and_then(|value| value.to_str().ok());
    let host_ok = host == Some(address.as_str())
        || host == Some(format!("localhost:{}", app.address.port()).as_str());
    let authorized = is_mcp && local_client;
    if !host_ok
        || headers.get(header::ORIGIN).is_some_and(|value| value != origin.as_str())
        || headers.get("sec-fetch-site").is_some_and(|value| value != "same-origin" && value != "none")
        || (!local_client && is_mcp && !authorized)
        || (!local_client && is_api && !is_media && !is_project_events && !is_backup_download && headers.get("x-dramlo-ui").is_none_or(|value| value != "1"))
    {
        if is_mcp {
            return (StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                Json(Failure::new(StatusCode::FORBIDDEN, "Dramlo 本机请求验证失败".into()).payload())).into_response();
        }
        return (StatusCode::FORBIDDEN, "Dramlo 本机请求验证失败").into_response();
    }
    let mut response = next.run(request).await;
    if response.headers().get(header::CONTENT_TYPE).is_some_and(|value| value == "application/json") {
        response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
    }
    if is_app_shell {
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    response
}

#[derive(Deserialize)]
struct LocalFile {
    path: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Command {
    command: String,
    path: Option<PathBuf>,
    value: Option<String>,
    expected_updated_at: Option<u64>,
    expected_value: Option<Value>,
    compare_value: Option<bool>,
    name: Option<String>,
    id: Option<String>,
}

impl Command {
    fn path(&self, app: &App) -> Result<PathBuf> {
        scoped_path(&app.root, self.path.as_deref().ok_or_else(|| invalid("缺少文件路径"))?)
    }
}

async fn file_read(State(app): State<App>, Query(options): Query<LocalFile>, request: Request) -> Result<Response> {
    let target = scoped_path(&app.root, &options.path)?;
    if !target.is_file() {
        return Err(Failure::new(StatusCode::NOT_FOUND, "文件未找到".into()));
    }
    if target.extension().and_then(|value| value.to_str()).is_none_or(|value| {
        !["png", "jpg", "jpeg", "webp", "gif", "mp4", "webm", "mov", "wav", "mp3", "m4a", "ogg", "flac", "bin"]
            .iter().any(|item| value.eq_ignore_ascii_case(item))
    }) {
        return Err(invalid("文件类型不支持流式访问"));
    }
    Ok(ServeFile::new(target).oneshot(request).await.unwrap().map(Body::new))
}

async fn file_write(State(app): State<App>, Query(options): Query<LocalFile>, contents: Bytes) -> Result<Json<Value>> {
    let target = scoped_path(&app.root, &options.path)?;
    if !target.strip_prefix(&app.root).is_ok_and(|relative|
        relative.components().any(|part| matches!(part.as_os_str().to_str(), Some("assets" | "thumbs" | "library")))) {
        return Err(invalid("二进制内容只能保存为 Dramlo 素材"));
    }
    if target.extension().is_none() || target.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json")) {
        return Err(invalid("素材文件名无效"));
    }
    let _lock = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
    let relative = target.strip_prefix(&app.root).map_err(|_| invalid("文件不在 Dramlo 工作区"))?;
    if let Some(folder) = relative.components().next().map(|part| part.as_os_str()) {
        if folder != "_library" && relative.components().count() >= 3 && !app.root.join(folder).is_dir() {
            return Err(Failure::new(StatusCode::NOT_FOUND, "素材所属目录已移除，请重新打开项目".into()));
        }
    }
    atomic_save(&target, &contents)?;
    Ok(Json(Value::Null))
}

fn state_directory(root: &Path) -> Result<PathBuf> {
    let dir = root.join(".dramlo");
    if fs::symlink_metadata(&dir).is_ok() && !normalize_root(&dir)?.starts_with(root) {
        return Err(invalid("配置目录指向工作区外部"));
    }
    Ok(dir)
}

fn kv_path(app: &App, key: &str) -> Result<PathBuf> {
    if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_control) {
        return Err(invalid("配置键无效"));
    }
    let hash = key.as_bytes().iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    let dir = state_directory(&app.root)?;
    let file = dir.join(format!("{hash:016x}.json"));
    if fs::symlink_metadata(&file).is_ok() && !normalize_root(&file)?.starts_with(&app.root) {
        return Err(invalid("配置文件指向工作区外部"));
    }
    Ok(file)
}

fn save_text(options: &Command, app: &App) -> Result<Value> {
    let target = options.path(app)?;
    let text = options.value.as_deref().ok_or_else(|| invalid("缺少文件内容"))?;
    if is_asset_manifest(&target) && target.exists() {
        return Err(invalid("请使用素材清单增量操作"));
    }
    if target.extension().and_then(|extension| extension.to_str()).is_none_or(|ext|
        !["json", "md", "txt", "srt"].iter().any(|allowed| ext.eq_ignore_ascii_case(allowed))) {
        return Err(invalid("不支持的文本文件类型"));
    }
    if target.file_name().is_some_and(|name| name == "project.json") {
        let input: Value = serde_json::from_str(text).map_err(|_| invalid("项目 JSON 无效"))?;
        studio_mcp::validate_project_document(&input)?;
        let revision = input.get("updatedAt").and_then(Value::as_u64).ok_or_else(|| invalid("项目 JSON 缺少修订号"))?;
        if target.exists() {
            let previous: Value = serde_json::from_slice(&fs::read(&target)?).map_err(|_| invalid("现有项目 JSON 损坏"))?;
            let current = previous.get("updatedAt").and_then(Value::as_u64);
            if current != options.expected_updated_at || current.is_some_and(|stamp| stamp > revision)
                || previous.get("id") != input.get("id") {
                return Err(Failure::new(StatusCode::CONFLICT, "项目已更新，请重新打开后再保存".into()));
            }
            generation_review::validate_frame_changes(app,&target,&previous,&input)?;
        } else {
            if options.expected_updated_at.is_some() || target.parent().is_none_or(|directory| !directory.is_dir()) {
                return Err(Failure::new(StatusCode::NOT_FOUND, "项目已移除，无法保存旧页面中的修改".into()));
            }
            generation_review::validate_frame_changes(app,&target,&Value::Null,&input)?;
            if let Some(folder) = target.parent().filter(|folder| folder.parent() == Some(app.root.as_path())) {
                project_authoring::initialize_project_instructions(folder)?;
            }
        }
    }
    atomic_save(&target, text.as_bytes())?;
    Ok(Value::Null)
}

fn is_asset_manifest(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("manifest.json"))
        && path.parent().and_then(Path::file_name).is_some_and(|name| name.eq_ignore_ascii_case("assets"))
        && path.parent().and_then(Path::parent).is_some_and(|dir| dir.join("project.json").is_file())
}

fn patch_asset_manifest(options: &Command, app: &App) -> Result<Value> {
    let path = options.path(app)?;
    if !is_asset_manifest(&path) { return Err(invalid("只支持项目素材清单")); }
    let id = options.id.as_deref().filter(|id| !id.trim().is_empty()).ok_or_else(|| invalid("缺少素材 ID"))?;
    let mut items: Vec<Value> = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("素材清单无效"))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => vec![],
        Err(error) => return Err(error.into()),
    };
    if let Some(raw) = &options.value {
        let asset: Value = serde_json::from_str(raw).map_err(|_| invalid("素材条目无效"))?;
        if asset["id"] != id { return Err(invalid("素材 ID 不一致")); }
        if let Some(existing) = items.iter_mut().find(|item| item["id"] == id) { *existing = asset; }
        else { items.insert(0,asset); }
    } else { items.retain(|item| item["id"] != id); }
    atomic_save(&path,&serde_json::to_vec_pretty(&items).map_err(|_| invalid("无法写入素材清单"))?)?;
    Ok(json!(items))
}

async fn handler(State(app): State<App>, Json(options): Json<Command>) -> Result<Json<Value>> {
    let _lock = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
    let value = match options.command.as_str() {
        "getRoot" => json!(&app.root),
        "getActiveProject" => studio_mcp::active_project(&app)?.map_or(Value::Null, |(_, project)|
            json!({ "projectId": project["id"], "updatedAt": project["updatedAt"] })),
        "setActiveProject" => studio_mcp::set_active_project(&app, options.id.as_deref().ok_or_else(|| invalid("缺少项目 ID"))?)?,
        "listProjectDirs" => {
            let mut projects = Vec::new();
            for entry in fs::read_dir(&app.root)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() && scoped_path(&app.root, &entry.path()).is_ok() &&
                    scoped_path(&app.root, &entry.path().join("project.json")).is_ok_and(|file| file.is_file()) {
                    projects.push(entry.path());
                }
            }
            json!(projects)
        }
        "ensureProjectDir" => {
            let id = options.id.as_deref().filter(|id| !id.is_empty() && id.len() <= 100 && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte)))
                .ok_or_else(|| invalid("项目 ID 无效"))?;
            let name: String = options.name.unwrap_or_default().chars().filter(|character| {
                character.is_alphanumeric() || matches!(character, ' ' | '-' | '_')
            }).take(48).collect();
            let path = scoped_path(&app.root, Path::new(&format!("{}__{id}", name.trim())))?;
            fs::create_dir_all(&path)?;
            json!(path)
        }
        "readText" => json!(fs::read_to_string(options.path(&app)?)?),
        "writeText" => save_text(&options, &app)?,
        "patchAssetManifest" => patch_asset_manifest(&options, &app)?,
        "removeIndexed" => workspace::remove_indexed(&app, &options)?,
        "exists" => json!(options.path(&app)?.exists()),
        "openPath" => {
            let target = options.path(&app)?;
            if !target.exists() {
                return Err(Failure::new(StatusCode::NOT_FOUND, "文件未找到".into()));
            }
            let directory = if target.is_dir() { target.as_path() } else { target.parent().ok_or_else(|| invalid("没有所在目录"))? };
            #[cfg(windows)]
            std::process::Command::new("explorer.exe").arg(directory).spawn()?;
            #[cfg(not(windows))]
            std::process::Command::new("xdg-open").arg(directory).spawn()?;
            json!("")
        }
        "kvGet" | "kvSet" => {
            let key = options.name.as_deref().ok_or_else(|| invalid("缺少配置键"))?;
            let file = kv_path(&app, key)?;
            if options.command == "kvGet" {
                if !file.exists() { return Ok(Json(Value::Null)); }
                let entry: Value = serde_json::from_slice(&fs::read(file)?).map_err(|_| invalid("配置文件损坏"))?;
                if entry.get("key").and_then(Value::as_str) != Some(key) { return Err(invalid("配置键不匹配")); }
                entry.get("value").cloned().unwrap_or(Value::Null)
            } else {
                if options.compare_value == Some(true) {
                    let current = if file.exists() {
                        serde_json::from_slice::<Value>(&fs::read(&file)?)
                            .map_err(|_| invalid("配置文件损坏"))?["value"].clone()
                    } else { Value::Null };
                    if current != options.expected_value.unwrap_or(Value::Null) {
                        return Err(Failure::new(StatusCode::CONFLICT, "配置已被其他客户端更新，请重新打开工作台".into()));
                    }
                }
                let next = json!({ "key": key, "value": options.value });
                atomic_save(&file, next.to_string().as_bytes())?;
                Value::Null
            }
        }
        _ => return Err(invalid("未知 Dramlo 工作区命令")),
    };
    Ok(Json(value))
}

async fn session(State(app): State<App>) -> Json<Value> {
    Json(json!({
        "service": "dramlo-web",
        "mcpCommand": env::current_exe().ok(),
        "mcpFfmpegBin": env::var_os("DRAMLO_FFMPEG_BIN").map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.is_file()),
        "mcpLauncherToken": app.mcp_launcher_token,
        "root": app.root,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": if cfg!(windows) { "win32" } else { env::consts::OS },
    }))
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    if env::args_os().nth(1).is_some_and(|value| value == "mcp") {
        let mode = env::args().nth(2).unwrap_or_else(|| "studio".into());
        let launcher_token = env::args().nth(3).is_some_and(|flag| flag == "--token-from-launcher");
        return stdio_mcp::run(&mode, launcher_token).await.map_err(|error| error.1.into());
    }
    let root = PathBuf::from(args.next().ok_or("用法: dramlo-server <工作区目录> <前端 dist 目录> [端口]")?);
    let public = PathBuf::from(args.next().ok_or("缺少前端 dist 目录")?);
    let port = args.next().map(|port| port.into_string().map_err(|_| "端口无效")).transpose()?.unwrap_or_else(|| "38042".into()).parse::<u16>()?;
    if !public.join("index.html").is_file() { return Err("前端未构建，请先运行 npm run build".into()); }
    fs::create_dir_all(&root)?;
    let root = normalize_root(&root)?;
    let state_dir = state_directory(&root).map_err(|error| error.1)?;
    fs::create_dir_all(&state_dir)?;
    let lock_path = state_dir.join("server.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| !metadata.is_file() || metadata.is_symlink()) {
        return Err("工作区锁文件无效".into());
    }
    // Keep the same lock file across restarts. The OS releases the lock when the process exits.
    let workspace_lock = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path)?;
    match workspace_lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Err("此工作区已有 Dramlo Web 服务正在运行".into()),
        Err(fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let address = listener.local_addr()?;
    let mcp_launcher_token = args.next().is_some_and(|flag| flag == "--token-from-launcher");
    let (shutdown, mut stop) = tokio::sync::watch::channel(false);
    let app = App {
        root, address, writes: Arc::default(),
        active_project: Arc::default(), mcp_launcher_token, shutdown, project_revision: tokio::sync::watch::channel(None).0, video_active: Arc::default(), plan_active: Arc::default(), comfy_children: Arc::default(),
    };
    watch_project_revisions(app.clone());
    let router = Router::new()
        .route("/api/dramlo/session", get(session))
        .route("/api/dramlo/project-events", get(project_events))
        .route("/api/dramlo/comfy", post(video_queue::browser_comfy).layer(DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/dramlo/image-queue", post(image_queue::browser).layer(DefaultBodyLimit::max(12 * 1024 * 1024)))
        .route("/api/dramlo/execution-plan", post(execution_plan::browser).layer(DefaultBodyLimit::max(12 * 1024 * 1024)))
        .route("/api/dramlo/video-queue", post(video_queue::browser).layer(DefaultBodyLimit::max(256 * 1024)))
        .route("/api/dramlo/comfy-instances", post(managed_comfy::browser).layer(DefaultBodyLimit::max(128 * 1024)))
        .route("/api/dramlo/canvas-extensions", post(canvas_extensions::browser))
        .route("/api/dramlo/creative-skills", get(studio_mcp::local_skills_browser))
        .route("/api/dramlo/mcp-capabilities", get(studio_mcp::capabilities))
        .route("/api/dramlo/video-provider", post(remote_video::provider_api).layer(DefaultBodyLimit::max(64 * 1024 * 1024)))
        .route("/api/dramlo/remote-video", post(remote_video::download).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/api/dramlo/workspace-backup", get(workspace_backup::download).post(workspace_backup::upload))
        .route("/api/dramlo/workspace-actions", post(studio_mcp::browser_workspace))
        .route("/api/dramlo/mcp", post(studio_mcp::invoke).layer(DefaultBodyLimit::max(12 * 1024 * 1024)))
        .route("/api/dramlo/fs", post(handler).layer(DefaultBodyLimit::max(16 * 1024 * 1024)))
        .route("/api/dramlo/file", get(file_read).put(file_write).layer(DefaultBodyLimit::max(256 * 1024 * 1024)))
        .fallback_service(ServeDir::new(public))
        .layer(middleware::from_fn_with_state(app.clone(), guard))
        .layer(SetResponseHeaderLayer::if_not_present(header::HeaderName::from_static("cross-origin-opener-policy"), HeaderValue::from_static("same-origin")))
        .layer(SetResponseHeaderLayer::if_not_present(header::HeaderName::from_static("cross-origin-embedder-policy"), HeaderValue::from_static("require-corp")))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .with_state(app);
    println!("http://{address}");
    axum::serve(listener, router).with_graceful_shutdown(async move {
        let _ = stop.wait_for(|requested| *requested).await;
    }).await?;
    Ok(())
}
