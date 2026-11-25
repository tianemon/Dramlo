use super::*;
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const LIMIT: usize = 512 * 1024;
static PROCESSES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const PROTOCOLS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Provider {
    id: String,
    command: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    tools: Vec<String>,
    #[serde(default)]
    revoked: bool,
}

fn load(app: &App) -> Result<Vec<Provider>> {
    match fs::read(kv_path(app, "extension-mcp-providers")?) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("插件连接配置无效")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

fn identifier(id: &str) -> bool {
    !id.is_empty() && id.len() <= 96 && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}

pub(super) fn configure(app: &App, input: &Value) -> Result<Value> {
    if input["confirmed"] != true { return Err(invalid("请在扩展库确认运行权限")); }
    let mut provider: Provider = serde_json::from_value(input["provider"].clone()).map_err(|_| invalid("MCP 插件配置无效"))?;
    if !identifier(&provider.id) || !provider.command.is_absolute() || !provider.command.is_file() ||
        !provider.command.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) && cfg!(windows) ||
        provider.args.len() > 32 || provider.args.iter().any(|arg| arg.len() > 2048 || arg.contains('\0')) ||
        provider.tools.is_empty() || provider.tools.len() > 32 || provider.tools.iter().any(|item| !identifier(item)) {
        return Err(invalid("请填写本机可执行文件绝对路径及明确允许的工具名"));
    }
    provider.command = provider.command.canonicalize()?;
    provider.revoked = false;
    let _lock = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let mut providers = load(app)?;
    if providers.iter().any(|entry| entry.id == provider.id) {
        return Err(invalid("连接 ID 已存在，请使用新 ID 以免改变现有扩展的执行器"));
    }
    let id = provider.id.clone();
    providers.push(provider);
    atomic_save(&kv_path(app, "extension-mcp-providers")?, &serde_json::to_vec(&providers)
        .map_err(|_| invalid("无法保存 MCP 连接"))?)?;
    Ok(json!({ "id": id }))
}

pub(super) fn list(app: &App) -> Result<Value> {
    let providers = load(app)?;
    Ok(json!({ "items": providers.iter().filter(|provider| !provider.revoked).map(|provider| json!({
        "id": provider.id, "command": provider.command, "tools": provider.tools
    })).collect::<Vec<_>>() }))
}

pub(super) fn remove(app: &App, input: &Value) -> Result<Value> {
    if input["confirmed"] != true { return Err(invalid("请先确认撤销 MCP 连接")); }
    let id = input["providerId"].as_str().ok_or_else(|| invalid("缺少连接 ID"))?;
    let _lock = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let mut providers = load(app)?;
    let Some(provider) = providers.iter_mut().find(|provider| provider.id == id && !provider.revoked) else {
        return Ok(json!({ "removed": false }));
    };
    provider.revoked = true;
    provider.args.clear();
    atomic_save(&kv_path(app, "extension-mcp-providers")?, &serde_json::to_vec(&providers)
        .map_err(|_| invalid("无法保存 MCP 连接"))?)?;
    Ok(json!({ "removed": true }))
}

pub(super) fn resolve(app: &App, id: &str, tool: &str) -> Result<Provider> {
    load(app)?.into_iter().find(|provider| !provider.revoked && provider.id == id && provider.tools.iter().any(|name| name == tool))
        .ok_or_else(|| invalid("请在扩展库登记 MCP 连接并授权该工具"))
}

pub(super) async fn call(provider: &Provider, tool: &str, arguments: Value) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(30), exchange(provider, tool, Some(arguments))).await
        .map_err(|_| invalid("第三方 MCP 超时，调用结果未知。先核对上游状态"))?
}

pub(super) async fn describe(provider: &Provider, tool: &str) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(15), exchange(provider, tool, None)).await
        .map_err(|_| invalid("第三方 MCP 工具发现超时"))?
}

async fn exchange(provider: &Provider, tool: &str, arguments: Option<Value>) -> Result<Value> {
    let _permit = PROCESSES.try_acquire().map_err(|_| invalid("本机 MCP 插件正在处理其他请求，请稍后再试"))?;
    let mut command = tokio::process::Command::new(&provider.command);
    command.args(&provider.args).kill_on_drop(true).env_clear()
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(parent) = provider.command.parent() { command.current_dir(parent); }
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
        for key in ["SystemRoot", "WINDIR"] {
            if let Some(value) = env::var_os(key) { command.env(key, value); }
        }
    }
    let mut child = command.spawn().map_err(|_| invalid("无法启动第三方 MCP 进程"))?;
    let mut input = child.stdin.take().ok_or_else(|| invalid("MCP 输入不可用"))?;
    let mut output = BufReader::new(child.stdout.take().ok_or_else(|| invalid("MCP 输出不可用"))?);
    async fn send(input: &mut tokio::process::ChildStdin, value: Value) -> Result<()> {
        let mut bytes = value.to_string().into_bytes();
        bytes.push(b'\n');
        input.write_all(&bytes).await?;
        Ok(())
    }
    async fn receive(input: &mut tokio::process::ChildStdin, output: &mut BufReader<tokio::process::ChildStdout>, id: i64) -> Result<Value> {
        for _ in 0..32 {
            let mut bytes = Vec::new();
            let count = (&mut *output).take((LIMIT + 1) as u64).read_until(b'\n', &mut bytes).await?;
            if count == 0 || count > LIMIT { return Err(invalid("第三方 MCP 响应为空或过大")); }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid("第三方 MCP 响应不是 JSON-RPC"))?;
            if value["jsonrpc"] != "2.0" { return Err(invalid("第三方 MCP 协议无效")); }
            if value["method"].is_string() {
                if let Some(server_id) = value.get("id") {
                    send(input, json!({"jsonrpc":"2.0","id":server_id,"error":{"code":-32601,"message":"Method not supported"}})).await?;
                }
                continue;
            }
            if value["id"] != id { continue; }
            if !value["result"].is_object() { return Err(invalid("第三方 MCP 调用失败，请查看上游状态")); }
            return Ok(value["result"].clone());
        }
        Err(invalid("第三方 MCP 未返回对应响应"))
    }
    send(&mut input, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":PROTOCOLS[0],"capabilities":{},"clientInfo":{"name":"dramlo","version":"1"}
    }})).await?;
    let initialized = receive(&mut input, &mut output, 1).await?;
    if !initialized["protocolVersion"].as_str().is_some_and(|version| PROTOCOLS.contains(&version)) ||
        !initialized["capabilities"]["tools"].is_object() {
        return Err(invalid("第三方 MCP 没有提供兼容的工具能力"));
    }
    send(&mut input, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await?;
    let Some(arguments) = arguments else {
        let mut cursor = Value::Null;
        for id in 2..10 {
            let params = if cursor.is_string() { json!({"cursor":cursor}) } else { json!({}) };
            send(&mut input, json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":params})).await?;
            let tools = receive(&mut input, &mut output, id).await?;
            if let Some(entry) = tools["tools"].as_array().into_iter().flatten().find(|entry| entry["name"] == tool) {
                return Ok(entry.clone());
            }
            cursor = tools["nextCursor"].clone();
            if !cursor.is_string() { break; }
        }
        return Err(invalid("第三方 MCP 当前未发布此工具"));
    };
    send(&mut input, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":arguments}})).await?;
    let result = receive(&mut input, &mut output, 2).await?;
    if result["isError"] == true { return Err(invalid("第三方 MCP 工具报告失败")); }
    Ok(result)
}
