use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{multipart, redirect::Policy};
use std::time::Duration;

pub(super) const MAX_REFERENCES: usize = 10;
const MAX_REFERENCE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TOTAL_REFERENCE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn endpoint(profile: &Value) -> Result<reqwest::Url> {
    let raw = profile["baseUrl"].as_str().unwrap_or_default().trim().trim_end_matches('/');
    studio_mcp::model_address(raw)?;
    let url = reqwest::Url::parse(raw).map_err(|_| invalid("OpenAI Images 地址无效"))?;
    let loopback = url.host_str().is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !loopback {
        return Err(invalid("在线 OpenAI Images 接入需要 HTTPS"));
    }
    Ok(url)
}

pub(super) fn ready(profile: &Value) -> Result<()> {
    endpoint(profile)?;
    if !profile["model"].as_str().is_some_and(|model| !model.trim().is_empty()) {
        return Err(invalid("请填写 OpenAI Images 的模型 ID"));
    }
    credential(profile)?;
    Ok(())
}

fn credential(profile: &Value) -> Result<&str> {
    profile["apiKey"].as_str().map(str::trim)
        .filter(|value| !value.is_empty() && !value.chars().any(|ch| ch.is_whitespace() || ch.is_control()))
        .ok_or_else(|| invalid("请在图像配置中填写 OpenAI Images 密钥"))
}

pub(super) fn size(model: &str, ratio: (u32, u32)) -> (&'static str, (u32, u32)) {
    let custom = model.starts_with("gpt-image-2");
    match ratio {
        (16, 9) if custom => ("1536x864", (16, 9)),
        (9, 16) if custom => ("864x1536", (9, 16)),
        (4, 3) if custom => ("1280x960", (4, 3)),
        (3, 4) if custom => ("960x1280", (3, 4)),
        (21, 9) if custom => ("1792x768", (21, 9)),
        (1, 1) => ("1024x1024", (1, 1)),
        (w, h) if w < h => ("1024x1536", (2, 3)),
        _ => ("1536x1024", (3, 2)),
    }
}

fn reference_size(path: &Path) -> Result<u64> {
    let size = fs::metadata(path)?.len();
    if size == 0 || size > MAX_REFERENCE_BYTES {
        return Err(invalid("OpenAI Images 的单张参考图须大于 0 且不超过 20 MiB"));
    }
    Ok(size)
}

pub(super) fn check_references(paths: &[PathBuf]) -> Result<()> {
    use std::io::Read;
    let mut total = 0;
    for path in paths {
        total += reference_size(path)?;
        if total > MAX_TOTAL_REFERENCE_BYTES { return Err(invalid("OpenAI Images 参考图总大小不能超过 100 MiB")); }
        let mut header = [0; 12];
        fs::File::open(path)?.read_exact(&mut header).map_err(|_| invalid("参考图文件无效"))?;
        image_queue::image_format(&header)?;
    }
    Ok(())
}

pub(super) async fn request(profile: &Value, prompt: &str, size: &str, paths: &[PathBuf]) -> Result<reqwest::Request> {
    let base = endpoint(profile)?;
    let base = base.as_str().trim_end_matches('/');
    let suffix = if paths.is_empty() { "generations" } else { "edits" };
    let route = if base.ends_with("/v1") { format!("{base}/images/{suffix}") } else { format!("{base}/v1/images/{suffix}") };
    let model = profile["model"].as_str().unwrap_or_default().trim();
    let mut request = reqwest::Request::new(reqwest::Method::POST,
        reqwest::Url::parse(&route).map_err(|_| invalid("OpenAI Images 请求地址无效"))?);
    let auth = format!("Bearer {}", credential(profile)?);
    let mut auth = reqwest::header::HeaderValue::from_str(&auth).map_err(|_| invalid("OpenAI Images 密钥格式无效"))?;
    auth.set_sensitive(true);
    request.headers_mut().insert(reqwest::header::AUTHORIZATION, auth);
    request.headers_mut().insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("application/json"));

    let builder = reqwest::RequestBuilder::from_parts(client()?.clone(), request);
    let builder = if paths.is_empty() {
        builder.json(&json!({"model": model, "prompt": prompt, "size": size, "n": 1}))
    } else {
        let mut form = multipart::Form::new()
            .text("model", model.to_owned()).text("prompt", prompt.to_owned())
            .text("size", size.to_owned()).text("n", "1");
        let mut total = 0;
        for (index, path) in paths.iter().enumerate() {
            reference_size(path)?;
            let bytes = tokio::fs::read(path).await?;
            total += bytes.len() as u64;
            if bytes.len() as u64 > MAX_REFERENCE_BYTES || total > MAX_TOTAL_REFERENCE_BYTES {
                return Err(invalid("OpenAI Images 参考图超过请求大小限制"));
            }
            let (ext, mime) = image_queue::image_format(&bytes)?;
            let part = multipart::Part::bytes(bytes).file_name(format!("reference-{}.{ext}", index + 1))
                .mime_str(mime).map_err(|_| invalid("参考图格式无效"))?;
            form = form.part("image[]", part);
        }
        builder.multipart(form)
    };
    builder.build().map_err(|_| invalid("无法构建 OpenAI Images 请求"))
}

fn client() -> Result<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<std::result::Result<reqwest::Client, reqwest::Error>> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| reqwest::Client::builder()
        .redirect(Policy::none()).retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(300)).build())
        .as_ref().map_err(|error| invalid(&format!("无法初始化 OpenAI Images HTTPS 客户端：{error}")))
}

pub(super) async fn send(request: reqwest::Request) -> Result<reqwest::Response> {
    client()?.execute(request).await.map_err(|error| {
        let kind = if error.is_timeout() { "超时" }
            else if error.is_connect() { "连接失败" }
            else if error.is_request() { "请求构造失败" }
            else if error.is_body() { "请求体错误" }
            else { "请求失败" };
        invalid(&format!("OpenAI Images {kind}：{error}，请核对网络、代理与供应商状态"))
    })
}

pub(super) async fn read_image(mut reply: reqwest::Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = reply.chunk().await.map_err(|error| invalid(&format!("OpenAI Images 响应读取中断：{error}")))? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES { return Err(invalid("OpenAI Images 响应超过大小限制")); }
        body.extend_from_slice(&chunk);
    }
    let response: Value = serde_json::from_slice(&body).map_err(|_| invalid("OpenAI Images 响应不是有效 JSON"))?;
    let encoded = response["data"][0]["b64_json"].as_str()
        .ok_or_else(|| invalid("OpenAI Images 响应缺少 data[0].b64_json"))?;
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid("OpenAI Images 返回的图片编码无效"))?;
    image_queue::image_format(&bytes)?;
    Ok(bytes)
}

pub(super) async fn diagnose_error(mut reply: reqwest::Response) -> (Value, String) {
    let status = reply.status().as_u16();
    let request_id = reply.headers().get("x-request-id").and_then(|v| v.to_str().ok())
        .filter(|id| id.starts_with("req_") && id.len() <= 80 &&
            id.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')).map(str::to_owned);

    const MAX_ERROR_BYTES: usize = 16 * 1024;
    let mut bytes = Vec::new();
    let error = loop {
        match reply.chunk().await {
            Ok(Some(part)) if bytes.len() + part.len() <= MAX_ERROR_BYTES => bytes.extend_from_slice(&part),
            Ok(None) => break serde_json::from_slice::<Value>(&bytes).ok(),
            _ => break None,
        }
    };
    let error = error.unwrap_or_default();
    let details = [&error["error"], &error["error"]["detail"], &error["detail"]];
    let known_codes = ["moderation_blocked", "content_policy_violation", "safety_violation",
        "invalid_value", "invalid_parameter", "invalid_request_error", "invalid_image",
        "invalid_image_format", "image_too_large", "unsupported_value", "missing_required_parameter",
        "model_not_found", "invalid_api_key", "insufficient_quota", "rate_limit_exceeded"];
    let code = details.iter().filter_map(|detail| detail["code"].as_str()).find(|code| known_codes.contains(code));
    let param = details.iter().filter_map(|detail| detail["param"].as_str()).find(|param| {
        ["model","prompt","image","image[]","size","quality","n","output_format","background","images"].contains(param)
            || param.strip_prefix("image[").or_else(||param.strip_prefix("images["))
                .and_then(|index|index.strip_suffix(']')).is_some_and(|index|index.parse::<u8>().is_ok())
    });
    let (category, reason) = match (code,status) {
        (Some("moderation_blocked" | "content_policy_violation" | "safety_violation"),_) =>
            ("moderation","供应商的内容安全审核未通过，请核对提示词和这次提交的参考图"),
        (Some("invalid_image" | "invalid_image_format" | "image_too_large"),_) =>
            ("invalid_image","供应商拒绝参考图，请检查格式、尺寸和大小"),
        (Some("model_not_found"),_) => ("model","供应商未找到模型或当前账号无权使用"),
        (Some("insufficient_quota"),_) => ("quota","供应商额度不足"),
        (Some("invalid_api_key"),_) | (_,401 | 403) => ("authentication","供应商拒绝鉴权或访问权限"),
        (Some("rate_limit_exceeded"),_) | (_,429) => ("rate_limit","供应商请求频率受限"),
        (Some(_),_) | (_,400 | 422) => ("invalid_request","供应商拒绝请求参数。请核对模型和输入规格"),
        (_,408 | 500..=599) => ("uncertain","供应商未确认请求结果，请先核对原任务"),
        _ => ("provider_error","供应商返回异常响应，请核对原请求")
    };
    let mut message = format!("OpenAI Images HTTP {status}：{reason}");
    if let Some(code) = code { message.push_str(&format!("（{code}）")); }
    if let Some(param) = param { message.push_str(&format!("。参数：{param}")); }
    if let Some(id) = &request_id { message.push_str(&format!("。供应商请求 ID：{id}")); }
    (json!({"provider":"openai-images","httpStatus":status,"category":category,
        "code":code,"param":param,"requestId":request_id,"detail":reason}),message)
}
