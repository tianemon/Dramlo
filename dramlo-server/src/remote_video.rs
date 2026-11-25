use super::*;
use std::{net::IpAddr, time::Duration};

const LIMIT: u64 = 256 * 1024 * 1024;

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a,b,_,_] = ip.octets();
            a > 0 && a < 224 && !(a == 100 && (64..=127).contains(&b))
                && !matches!((a,b),(192,0)|(198,18)|(198,19))
                && !ip.is_private() && !ip.is_loopback() && !ip.is_link_local() && !ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            let groups = ip.segments();
            (groups[0] & 0xe000) == 0x2000 &&
                !matches!(groups[0],0x2001|0x2002|0x3fff)
        }
    }
}

fn configured_origin(app: &App, url: &reqwest::Url) -> bool {
    let Ok((_, settings)) = managed_comfy::settings(app) else { return false; };
    let same = |raw: &Value| raw.as_str().and_then(|value| reqwest::Url::parse(value).ok())
        .is_some_and(|base| base.origin()==url.origin());
    same(&settings["zealmanBaseUrl"]) ||
        settings["videos"].as_array().into_iter().flatten().any(|profile|same(&profile["baseUrl"]))
}

async fn client(app: &App, url: &reqwest::Url) -> Result<reqwest::Client> {
    let host = url.host_str().filter(|host| !host.is_empty()).ok_or_else(||invalid("视频地址无效"))?;
    if !url.username().is_empty() || url.password().is_some() { return Err(invalid("视频地址不能含凭据")); }
    let port = url.port_or_known_default().ok_or_else(||invalid("视频地址协议无效"))?;
    let local = matches!(host,"localhost"|"127.0.0.1"|"[::1]");
    if local {
        if url.scheme() != "http" || !configured_origin(app,url) {
            return Err(invalid("本机视频地址不属于已保存的接入"));
        }
    } else if url.scheme()!="https" || port!=443 {
        return Err(invalid("远程原片需要 HTTPS 地址"));
    }
    let mut builder = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(120));
    if !local {
        if let Ok(ip) = host.trim_matches(['[',']']).parse::<IpAddr>() {
            if !public_address(ip) { return Err(invalid("远程视频地址不是公共地址")); }
        } else {
            let addresses: Vec<_> = tokio::time::timeout(Duration::from_secs(8),tokio::net::lookup_host((host,port)))
                .await.map_err(|_|invalid("视频主机解析超时"))?.map_err(|_|invalid("无法解析视频主机"))?.collect();
            if addresses.is_empty() || addresses.iter().any(|addr| !public_address(addr.ip())) {
                return Err(invalid("远程视频地址解析到了非公共地址"));
            }
            builder = builder.resolve_to_addrs(host,&addresses);
        }
    } else if host == "localhost" {
        builder = builder.resolve(host,SocketAddr::from(([127,0,0,1],port)));
    }
    builder.build().map_err(|_|invalid("无法初始化视频下载"))
}

pub(super) async fn download(State(app): State<App>, Json(input): Json<Value>) -> Result<Response> {
    let raw = input["url"].as_str().filter(|url| url.len() <= 8192).ok_or_else(|| invalid("视频地址无效"))?;
    let mut url = reqwest::Url::parse(raw).map_err(|_|invalid("视频地址无效"))?;
    for attempt in 0..=4 {
        let response = client(&app,&url).await?.get(url.clone()).send().await
            .map_err(|_|invalid("供应商视频下载失败"))?;
        if response.status().is_redirection() {
            if attempt==4 { return Err(invalid("视频下载跳转次数过多")); }
            let location = response.headers().get(header::LOCATION).and_then(|value|value.to_str().ok())
                .ok_or_else(||invalid("视频跳转没有可用地址"))?;
            let next = url.join(location).map_err(|_|invalid("视频跳转地址无效"))?;
            if url.scheme() == "https" && next.scheme()!="https" { return Err(invalid("视频地址跳转到非 HTTPS 位置")); }
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(invalid(&format!("供应商原片下载返回 HTTP {}",response.status().as_u16())));
        }
        if response.content_length().is_some_and(|size|size > LIMIT) { return Err(invalid("原片超过 256 MB")); }
        let mut response = response;
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_|invalid("读取原片失败"))? {
            if data.len() as u64 + chunk.len() as u64 > LIMIT { return Err(invalid("原片超过 256 MB")); }
            data.extend_from_slice(&chunk);
        }
        let mime = if data.get(4..8)==Some(b"ftyp") { "video/mp4" }
            else if data.starts_with(&[0x1a,0x45,0xdf,0xa3]) { "video/webm" }
            else { return Err(invalid("供应商下载结果不是 MP4 或 WebM")); };
        return Ok(([(header::CONTENT_TYPE,mime)], data).into_response());
    }
    Err(invalid("未能读取供应商原片"))
}

pub(super) async fn provider_api(State(app): State<App>, Json(input): Json<Value>) -> Result<Json<Value>> {
    let raw = input["url"].as_str().filter(|url| url.len() <= 8192).ok_or_else(|| invalid("供应商地址无效"))?;
    let url = reqwest::Url::parse(raw).map_err(|_| invalid("供应商地址无效"))?;
    if !configured_origin(&app,&url) { return Err(invalid("请先保存此供应商接入")); }
    let route = url.path().trim_end_matches('/');
    let method = input["method"].as_str().unwrap_or("GET");
    let allowed = match method {
        "POST" => route.contains("/comfyui_workflow/") || route.ends_with("/api/workflow/generate")
            || route.ends_with("/services/aigc/video-generation/video-synthesis"),
        "GET" => route.contains("/comfyui_workflow/") || route.contains("/tasks/")
            || route.starts_with("/api/workflow/"),
        _ => false,
    };
    if !allowed { return Err(invalid("该路径不属于已支持的视频服务接口")); }
    let client = client(&app,&url).await?;
    let mut request = if method == "GET" { client.get(url) } else { client.post(url) };
    if let Some(auth) = input["authorization"].as_str().filter(|auth| !auth.is_empty()) { request = request.header(header::AUTHORIZATION,auth); }
    for key in ["x-dashscope-async","x-dashscope-ossresource-resolve"] {
        if let Some(value) = input["headers"].as_object().into_iter().flatten()
            .find(|(name,_)| name.eq_ignore_ascii_case(key)).and_then(|(_,value)|value.as_str()) {
            request = request.header(key,value);
        }
    }
    if method == "POST" {
        let body = input["body"].as_str().ok_or_else(||invalid("缺少视频任务参数"))?;
        request = request.header(header::CONTENT_TYPE,"application/json").body(body.to_owned());
    }
    let mut response = request.send().await.map_err(|_| invalid("视频服务请求失败，请核对原任务后再试"))?;
    let status = response.status();
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_|invalid("读取视频服务响应失败"))? {
        if data.len() + chunk.len() > 2 * 1024 * 1024 { return Err(invalid("视频服务响应过大")); }
        data.extend_from_slice(&chunk);
    }
    Ok(Json(json!({"status":status.as_u16(),"ok":status.is_success(),"text":String::from_utf8_lossy(&data)})))
}
