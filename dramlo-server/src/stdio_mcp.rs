use super::*;
use std::io::Write;
use tokio::io::{AsyncBufReadExt, BufReader};
use std::process::{Command as Process, Stdio};
use std::time::Duration;

const PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
pub(super) const STDIO_ONLY_TOOLS: [(&str,&str); 2] = [
    ("studio_web_status", "检查 Dramlo Web 服务"),
    ("studio_start_web", "启动指定的本机 Dramlo 工作区"),
];

const COMPACT_NATIVE: &[&str] = &["studio_web_status", "studio_project", "studio_pending_jobs"];

fn compact_tools() -> Vec<Value> {
    let mut tools: Vec<_> = studio_mcp::schemas().into_iter()
        .filter(|tool| tool["name"].as_str().is_some_and(|name| COMPACT_NATIVE.contains(&name)))
        .collect();
    tools.push(json!({"name":"studio_web_status","description":"检查 Dramlo 连接；尚未启动时通过 studio_find 查找 studio_start_web。",
        "inputSchema":{"type":"object","additionalProperties":false,"properties":{}}}));
    tools.push(json!({"name":"studio_find","description":"查找 Dramlo 操作：剧本、分镜、画布、素材、生成、ComfyUI、设置、导出和创作 Skill。按目标查少量操作，按完整工具名获取参数定义。调用用 studio_call。",
        "inputSchema":{"type":"object","additionalProperties":false,"properties":{
            "query":{"type":"string","minLength":1,"maxLength":200,"description":"任务关键词，或完整工具名。"},
            "includeDefinition":{"type":"boolean","description":"关键词查询时附最佳匹配的参数定义。"}},"required":["query"]}}));
    tools.push(json!({"name":"studio_call","description":"按 Dramlo 工具名执行一个操作，使用该操作自己的参数。名称或参数不清楚时先用 studio_find。",
        "inputSchema":{"type":"object","additionalProperties":false,"properties":{
            "name":{"type":"string","minLength":1,"description":"studio_find 返回的工具名。"},
            "arguments":{"type":"object","description":"目标操作的参数。"}},"required":["name","arguments"]}}));
    tools
}

const SEARCH_HINTS: &[(&str, &[&str])] = &[
    ("studio_start_web", &["启动", "冷启动", "打开 Dramlo"]),
    ("studio_get_guide", &["指南", "手册", "教程", "怎么操作"]),
    ("studio_list_skills", &["skill", "技能", "创作手册"]),
    ("studio_read_skill", &["skill", "技能", "创作手册"]),
    ("studio_shots", &["镜头", "分镜", "读取镜头", "人物", "场景", "道具"]),
    ("studio_batch_edit_shots", &["修改分镜", "镜头标题", "改名", "剪点", "切入拍点", "观众新知", "站位", "视线", "道具焦点"]),
    ("studio_project_items", &["人物", "场景", "道具", "素材", "资产卡"]),
    ("studio_update_shot_inputs", &["换绑", "换人物", "换场景", "绑定", "连线", "解绑", "道具"]),
    ("studio_canvas_graph", &["画布", "连线", "读回", "节点"]),
    ("studio_comfy_discover", &["comfyui", "cfui", "检测", "发现实例"]),
    ("studio_comfy_workflows", &["comfyui", "cfui", "浏览工作流"]),
    ("studio_comfy_workflow_read", &["comfyui", "工作流", "尾帧", "槽位"]),
    ("studio_comfy_import", &["comfyui", "cfui", "导入工作流", "尾帧"]),
    ("studio_prepare_video", &["预检", "出片", "视频", "尾帧"]),
    ("studio_prepare_image", &["生图", "预检", "出图"]),
];

fn find_tools(tools: &[Value], arguments: &Value) -> Result<Value> {
    let query = arguments["query"].as_str().map(str::trim).filter(|query| !query.is_empty())
        .ok_or_else(|| invalid("请提供任务关键词或完整工具名"))?.to_lowercase();
    if let Some(tool) = tools.iter().find(|tool| tool["name"] == query.as_str()) {
        return Ok(json!({"definition":tool}));
    }
    let tokens: std::collections::HashSet<_> = query
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|word| !word.is_empty())
        .flat_map(|word| {
            let letters: Vec<_> = word.chars().collect();
            if letters.iter().any(|ch| ch.is_alphabetic() && !ch.is_ascii()) {
                letters.windows(2).map(|chunk| chunk.iter().collect()).collect::<Vec<String>>()
            } else {
                vec![word.to_owned()]
            }
        }).collect();
    let mut matches: Vec<_> = tools.iter().filter_map(|tool| {
        let name = tool["name"].as_str()?;
        let description = tool["description"].as_str().unwrap_or_default().to_lowercase();
        let hints = SEARCH_HINTS.iter().find(|(id, _)| *id == name).map(|(_, hints)| *hints).unwrap_or_default();
        let weight: usize = hints.iter().filter(|hint| query.contains(**hint)).map(|hint| hint.chars().count()).sum();
        let score = weight * 3 + tokens.iter().filter(|term| name.contains(term.as_str()) || description.contains(term.as_str())).count();
        (score > 0).then_some((score, name, tool))
    }).collect();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    let items: Vec<_> = matches.iter().take(5).map(|(_, name, tool)| {
        let summary: String = tool["description"].as_str().unwrap_or_default().chars().take(100).collect();
        json!({"name":name,"description":summary})
    }).collect();
    let mut result = json!({"items":items});
    if arguments["includeDefinition"] == true {
        if let Some((_, _, tool)) = matches.first() { result["definition"] = (*tool).clone(); }
    }
    Ok(result)
}

fn validate_arguments(tool: &Value, arguments: &Value) -> Result<()> {
    let validator = jsonschema::options().should_validate_formats(true).build(&tool["inputSchema"])
        .map_err(|_| invalid("Dramlo 工具参数定义无效"))?;
    if let Some(error) = validator.iter_errors(arguments).next() {
        let path = error.instance_path.to_string();
        return Err(invalid(&format!("参数 {} 不符合工具定义，请用 studio_find 查看对应参数", if path.is_empty() { "/" } else { &path })));
    }
    Ok(())
}

struct Studio {
    origin: String,
    launcher_token: bool,
    address: SocketAddr,
}

impl Studio {
    fn new(launcher_token: bool) -> Result<Self> {
        let configured = env::var("DRAMLO_WEB_MCP_URL").unwrap_or_else(|_| "http://127.0.0.1:38042".into());
        let rest = configured.trim_end_matches('/').strip_prefix("http://")
            .ok_or_else(|| invalid("DRAMLO_WEB_MCP_URL 必须写成 http://127.0.0.1:38042 或 http://localhost:38042"))?;
        let (host, port) = rest.rsplit_once(':')
            .ok_or_else(|| invalid("DRAMLO_WEB_MCP_URL 缺少端口，例如 http://127.0.0.1:38042"))?;
        if host != "127.0.0.1" && host != "localhost" {
            return Err(invalid("DRAMLO_WEB_MCP_URL 只支持本机地址 127.0.0.1 或 localhost"));
        }
        let port = port.parse::<u16>().ok().filter(|port| *port != 0)
            .ok_or_else(|| invalid("DRAMLO_WEB_MCP_URL 端口无效"))?;
        Ok(Self {
            // 统一归一化到 IPv4 回环，避免 localhost 解析成 ::1 后连不上只绑 IPv4 的服务端。
            origin: format!("http://127.0.0.1:{port}"),
            launcher_token,
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        })
    }

    /// 服务端已不校验令牌：取不到就用空串，保证没有 DPAPI 文件的外部 Agent 也能调用。
    fn token(&self, create: bool) -> String {
        if self.launcher_token {
            launcher_token::launcher(create).ok().flatten()
        } else { launcher_token::configured() }.unwrap_or_default()
    }

    fn relay(origin: &str, token: &str) -> Value {
        json!({ "baseUrl": origin, "apiKey": token, "adapter": { "type": "comfyui-prompt" } })
    }

    async fn call(&self, name: &str, arguments: &Value) -> Result<Value> {
        if matches!(name, "studio_web_status" | "studio_start_web") {
            let mut capabilities = self.capabilities().await?;
            let reused = capabilities.is_some();
            if !reused && name == "studio_start_web" {
                if !matches!(tokio::net::TcpStream::connect(self.address).await, Err(error) if error.kind() == io::ErrorKind::ConnectionRefused) {
                    return Err(invalid("此端口已被占用，请检查工作台连接"));
                }
                self.start(&self.token(true))?;
                for _ in 0..30 {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    capabilities = self.capabilities().await?;
                    if capabilities.is_some() { break; }
                }
                if capabilities.is_none() { return Err(invalid("未等到 Web 工作台启动")); }
            }
            let ready = capabilities.is_some();
            let details = capabilities.unwrap_or_default();
            return Ok(json!({ "ready": ready, "reused": reused, "url": self.origin,
                "version": details["version"], "toolRevision": details["toolRevision"], "guideRevision": details["guideRevision"] }));
        }
        let (status, _, bytes) = video_queue::http(&Self::relay(&self.origin, &self.token(false)), "POST", "/api/dramlo/mcp", "application/json",
            json!({ "name": name, "input": if arguments.is_null() { json!({}) } else { arguments.clone() } }).to_string().into_bytes()).await?;
        let data = serde_json::from_slice::<Value>(&bytes);
        if !status.is_success() {
            let message = data.as_ref().ok().and_then(|value| value["error"].as_str()).unwrap_or("Dramlo 操作失败");
            let error = Failure::new(status, message.into());
            return Err(if let Ok(data) = &data { error.with_details(data) } else { error });
        }
        let data = data.map_err(|_| Failure::new(StatusCode::BAD_GATEWAY, "工作台响应无效".into()))?;
        if let Some(error) = data["error"].as_str() {
            return Err(Failure::new(StatusCode::BAD_REQUEST, error.into()));
        }
        Ok(data)
    }

    async fn capabilities(&self) -> Result<Option<Value>> {
        match tokio::net::TcpStream::connect(self.address).await {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let connection = Self::relay(&self.origin, &self.token(false));
        let request = video_queue::http(&connection, "GET", "/api/dramlo/mcp-capabilities", "application/json", Vec::new());
        let (status, _, bytes) = tokio::time::timeout(Duration::from_secs(2), request).await
            .map_err(|_| invalid("Dramlo 服务状态检查超时"))??;
        if !status.is_success() { return Err(invalid("无法验证此端口上的 Dramlo 服务")); }
        let details: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("Dramlo 能力响应无效"))?;
        if !details["tools"].is_array() { return Err(invalid("Dramlo 能力响应缺少工具列表")); }
        Ok(Some(details))
    }

    fn start(&self, token: &str) -> Result<()> {
        let root = env::var_os("DRAMLO_WEB_WORKSPACE").map(PathBuf::from)
            .filter(|root| root.is_absolute()).ok_or_else(|| invalid("请填写 DRAMLO_WEB_WORKSPACE 的绝对目录"))?;
        let binary = env::current_exe()?;
        let default = binary.parent().and_then(Path::parent).and_then(Path::parent).and_then(Path::parent)
            .map(|folder| folder.join("dist"));
        let embedded = binary.parent().map(|folder| folder.join("dist")).filter(|folder| folder.join("index.html").is_file());
        let frontend = env::var_os("DRAMLO_WEB_FRONTEND").map(PathBuf::from).or(embedded).or(default)
            .filter(|folder| folder.join("index.html").is_file()).ok_or_else(|| invalid("找不到前端目录，请设置 DRAMLO_WEB_FRONTEND"))?;
        let mut command = Process::new(binary);
        command.arg(root).arg(frontend).arg(self.address.port().to_string())
            .env("DRAMLO_MCP_BRIDGE_TOKEN", token)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if self.launcher_token { command.arg("--token-from-launcher"); }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command.spawn()?;
        Ok(())
    }

    fn tools(&self, capabilities: Option<&Value>) -> Vec<Value> {
        let schemas = studio_mcp::schemas();
        let mut entries = if let Some(details) = capabilities {
            details["toolDefinitions"].as_array().filter(|definitions| {
                definitions.iter().all(|tool| tool["name"].is_string() && tool["inputSchema"].is_object())
            }).cloned().unwrap_or_else(|| {
                schemas.iter().filter(|tool| details["tools"].as_array()
                    .is_some_and(|names| names.contains(&tool["name"]))).cloned().collect()
            })
        } else {
            schemas.iter().filter(|tool| tool["name"].as_str()
                .is_some_and(studio_mcp::native_method)).cloned().collect()
        };
        for (name, description) in STDIO_ONLY_TOOLS {
            entries.push(json!({ "name": name, "description": description, "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false } }));
        }
        entries
    }

    async fn compact_call(&self, name: &str, arguments: &Value) -> Result<Value> {
        let empty = json!({});
        let arguments = if arguments.is_null() { &empty } else { arguments };
        let public = compact_tools();
        let Some(definition) = public.iter().find(|tool| tool["name"] == name) else {
            return Err(invalid("请用 studio_find 查询工具，再用 studio_call 调用"));
        };
        validate_arguments(definition, arguments)?;
        if COMPACT_NATIVE.contains(&name) { return self.call(name, arguments).await; }
        let capabilities = self.capabilities().await?;
        let tools = self.tools(capabilities.as_ref());
        if name == "studio_find" { return find_tools(&tools, arguments); }
        let target = video_queue::field(arguments, "name")?;
        let definition = tools.iter().find(|tool| tool["name"] == target)
            .ok_or_else(|| invalid("找不到该操作，请用 studio_find 查询"))?;
        let input = &arguments["arguments"];
        validate_arguments(definition, input)?;
        if capabilities.is_none() && !STDIO_ONLY_TOOLS.iter().any(|(id, _)| *id == target) {
            return Err(invalid("Dramlo Web 尚未启动，请先调用 studio_start_web"));
        }
        self.call(target, input).await
    }
}

pub(super) async fn run(mode: &str, launcher_token: bool) -> Result<()> {
    let compact = mode == "studio-compact";
    let studio = if mode == "studio" || compact { Some(Studio::new(launcher_token)?) } else { None };
    if studio.is_none() && mode != "comfy" {
        return Err(invalid("用法: dramlo-server mcp studio-compact、mcp studio 或 mcp comfy"));
    }
    let mut output = io::stdout();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut listed_tools = None;
    let mut watch = tokio::time::interval(Duration::from_secs(2));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let line = tokio::select! {
            line = lines.next_line() => match line? { Some(line) => line, None => break },
            _ = watch.tick(), if listed_tools.is_some() => {
                if let Some(studio) = &studio {
                    if let Ok(Ok(Some(details))) = tokio::time::timeout(Duration::from_millis(500), studio.capabilities()).await {
                        let tools = studio.tools(Some(&details));
                        if listed_tools.as_ref() != Some(&tools) {
                            listed_tools = Some(tools);
                            json_line(&mut output, &json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}))?;
                        }
                    }
                }
                continue;
            }
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                json_line(&mut output, &json!({ "jsonrpc":"2.0", "id":null, "error":{"code":-32700,"message":"Parse error"} }))?;
                continue;
            }
        };
        let Some(id) = request.get("id") else { continue; };
        let operation = request["method"].as_str().unwrap_or_default();
        let params = &request["params"];
        let result = match operation {
            "initialize" => {
                let version = params["protocolVersion"].as_str().filter(|value| PROTOCOLS.contains(value)).unwrap_or(PROTOCOLS[0]);
                let mut result = json!({ "protocolVersion": version, "capabilities": { "tools": {} },
                    "serverInfo": { "name": format!("dramlo-{mode}"), "version": env!("CARGO_PKG_VERSION") } });
                if compact {
                    result["instructions"] = json!("先读取当前项目或任务状态。用 studio_find 搜索操作及参数，再用 studio_call 执行。指南和创作 Skill 按任务查找，正文只用于本次任务。");
                } else if studio.is_some() {
                    result["capabilities"]["tools"]["listChanged"] = json!(true);
                    result["instructions"] = json!(format!(
                        "用 studio_web_status 检查 Dramlo 连接。按任务用 studio_get_guide 读取指南，修订变化时重读。{}",
                        studio_mcp::SKILL_USAGE
                    ));
                }
                json!({ "result": result })
            }
            "ping" => json!({ "result": {} }),
            "tools/list" => {
                let tools = if compact { compact_tools() } else if let Some(studio) = &studio {
                    let capabilities = studio.capabilities().await.ok().flatten();
                    let tools = if let Some(details) = capabilities.as_ref() {
                        studio.tools(Some(details))
                    } else {
                        listed_tools.clone().unwrap_or_else(|| studio.tools(None))
                    };
                    listed_tools = Some(tools.clone());
                    tools
                } else { comfy_mcp::tools() };
                json!({ "result": { "tools": tools } })
            }

            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let args = &params["arguments"];
                let result = if let Some(studio) = &studio {
                    if compact { studio.compact_call(name, args).await } else { studio.call(name, args).await }
                }
                    else { comfy_mcp::call(name, args).await };
                match result {
                    Ok(data) => json!({ "result": { "content": [{ "type": "text", "text": data.to_string() }], "structuredContent": data } }),
                    Err(error) => json!({ "result": { "isError": true, "content": [{ "type": "text", "text": error.display_message() }],
                        "structuredContent": error.payload() } }),
                }
            }
            _ => json!({ "error": { "code": -32601, "message": "Method not found" } }),
        };
        let mut reply = result;
        reply["jsonrpc"] = json!("2.0");
        reply["id"] = id.clone();
        json_line(&mut output, &reply)?;
    }
    Ok(())
}

fn json_line(output: &mut impl Write, value: &Value) -> Result<()> {
    writeln!(output, "{value}")?;
    output.flush()?;
    Ok(())
}
