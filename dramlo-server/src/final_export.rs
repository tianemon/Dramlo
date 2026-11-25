use super::*;
use std::time::Duration;

pub(super) const METHODS: &[&str] = &["studio_export_final","studio_export_status"];

struct Clip {
    path: PathBuf,
    still: bool,
    start: f64,
    duration: f64,
    volume: f64,
}

fn export_size(project: &Value) -> (u32,u32) {
    match project["settings"]["aspectRatio"].as_str() {
        Some("9:16") => (720,1280), Some("1:1") => (1024,1024), Some("4:3") => (960,720),
        Some("21:9") => (1512,648), _ => (1280,720),
    }
}

fn enabled_clips(project: &Value) -> Vec<&Value> {
    let mut entries: Vec<_> = project["timeline"].as_array().into_iter().flatten()
        .filter(|clip| clip["enabled"] != false).collect();
    entries.sort_by(|a,b| a["startTime"].as_f64().unwrap_or_default().total_cmp(&b["startTime"].as_f64().unwrap_or_default()));
    entries
}

pub(super) fn timeline_issues(project: &Value) -> Vec<Value> {
    let mut issues = Vec::new();
    let mut previous = None;
    let mut expected = 0.0;
    for (index, clip) in enabled_clips(project).into_iter().enumerate() {
        let (Some(start),Some(duration)) = (clip["startTime"].as_f64(),clip["duration"].as_f64()) else {
            previous = None;
            continue;
        };
        let delta = start-expected;
        if (index == 0 || previous.is_some()) && delta.abs() > 0.001 + 1e-9 {
            let kind = if delta > 0.0 { "gap" } else { "overlap" };
            let label = if delta > 0.0 { "空隙" } else { "重叠" };
            let seconds = (delta.abs()*1000.0).round()/1000.0;
            let location = if index == 0 { "时间线起点与片段 1".into() }
                else { format!("片段 {} 与片段 {}", index, index+1) };
            issues.push(json!({"kind":kind,"afterClipId":previous,"clipId":clip["id"],"clipIndex":index+1,
                "seconds":seconds,"message":format!("{location}之间有 {seconds:.3} 秒{label}，导出前需修正")}));
        }
        previous = clip["id"].as_str();
        expected = start+duration;
    }
    issues
}

fn timeline_stamp(project: &Value) -> String {
    let entries = enabled_clips(project);
    let entries: Vec<_> = entries.into_iter().map(|clip| {
        let shot_audio = project["shots"].as_array().into_iter().flatten()
            .find(|shot| shot["id"] == clip["shotId"]).and_then(|shot| shot["audioAssetId"].as_str());
        json!([project_authoring::timeline_asset(project,clip),clip["startTime"].as_f64(),
            clip["duration"].as_f64(),clip["sourceIn"].as_f64(),clip["speed"].as_f64().unwrap_or(1.0),
            clip["volume"].as_f64().unwrap_or(1.0),clip["transitionIn"].as_str().unwrap_or("none"),
            clip["transitionDuration"].as_f64().unwrap_or_default(),clip["voiceAssetId"],shot_audio])
    }).collect();
    let snapshot = json!([entries,export_size(project),project["bgmClip"],project["textOverlays"]]);
    let hash = snapshot.to_string().bytes().fold(0xcbf29ce484222325u64, |hash, byte|
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3));
    format!("v1-{hash:016x}")
}

pub(super) fn stale(project: &Value) -> Option<bool> {
    let asset = project["finalVideoAssetId"].as_str()?;
    let record = &project["finalVideoExport"];
    if record["assetId"] != asset { return None; }
    let stamp = record["timelineStamp"].as_str()?.strip_prefix("v1-")?;
    Some(timeline_stamp(project).strip_prefix("v1-") != Some(stamp))
}

fn plan(app: &App, file: &Path, project: &Value, pending: &[String]) -> Result<(Vec<Clip>,(u32,u32),f64)> {
    if !project["bgmClip"].is_null() || project["textOverlays"].as_array().is_some_and(|items| !items.is_empty()) {
        return Err(invalid("服务端目前支持基础硬切。此时间线有配乐或叠字，请使用界面导出"));
    }
    let entries = enabled_clips(project);
    if entries.is_empty() || entries.len()>500 { return Err(invalid("导出需要 1 至 500 个启用片段")); }
    let issues = timeline_issues(project);
    if let Some(first) = issues.first() {
        let message = first["message"].as_str().unwrap_or_default();
        return Err(invalid(&format!("基础导出需要时间线连续：{message}")).with_issues(issues));
    }
    let mut clips = Vec::new();
    let mut total = 0.0;
    let mut media_issues = Vec::new();
    for (index, clip) in entries.into_iter().enumerate() {
        if !clip["startTime"].is_number() { return Err(invalid("时间线起点无效")); }
        let duration = clip["duration"].as_f64().filter(|value| *value > 0.0).ok_or_else(|| invalid("片段时长无效"))?;
        if clip["speed"].as_f64().unwrap_or(1.0) != 1.0 || clip["transitionIn"].as_str().unwrap_or("none") != "none" ||
            clip["voiceAssetId"].is_string() || project["shots"].as_array().into_iter().flatten()
                .any(|shot| shot["id"] == clip["shotId"] && shot["audioAssetId"].is_string()) {
            return Err(invalid("此片段使用了变速、转场或独立配音，请使用界面导出"));
        }
        let Some(id) = project_authoring::timeline_asset(project,clip) else {
            media_issues.push(json!({"kind":"missing-clip-media","clipId":clip["id"],"shotId":clip["shotId"],"clipIndex":index+1}));
            continue;
        };
        let (path,still) = match video_queue::media_path(app,file,id,"video") {
            Ok(path) => (path,false),
            Err(_) if pending.iter().any(|item| item == id) => (PathBuf::new(),false),
            Err(_) => (video_queue::media_path(app,file,id,"image")?,true),
        };
        let source_in = clip["sourceIn"].as_f64().filter(|value| *value >= 0.0).ok_or_else(|| invalid("片段入点无效"))?;
        if still && source_in != 0.0 { return Err(invalid("静帧片段的入点须为 0")); }
        let volume = clip["volume"].as_f64().unwrap_or(1.0);
        if !(0.0..=1.0).contains(&volume) { return Err(invalid("片段音量无效")); }
        total += duration;
        if total > 3600.0 { return Err(invalid("服务端单次最多导出 60 分钟")); }
        clips.push(Clip { path,still,start:source_in,duration,volume });
    }
    if let Some(first) = media_issues.first() {
        let first_index = first["clipIndex"].as_u64().unwrap_or_default();
        return Err(invalid(&format!("有 {} 个启用片段没有选用画面素材，首个是片段 {first_index}。请绑定素材或添加对应镜头的上游视频步骤", media_issues.len()))
            .with_issues(media_issues));
    }
    Ok((clips,export_size(project),total))
}

pub(super) fn inspect_plan(app: &App, file: &Path, project: &Value, pending: &[String]) -> Result<Value> {
    if super::ffmpeg_binary().is_err() {
        return Err(invalid("请配置本地 DRAMLO_FFMPEG_BIN 后再导出"));
    }
    let (clips,size,total) = plan(app,file,project,pending)?;
    Ok(json!({"clipCount":clips.len(),"duration":total,"width":size.0,"height":size.1}))
}

fn ffmpeg(binary: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command.kill_on_drop(true).args(["-nostdin","-hide_banner"]);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

async fn process(mut command: tokio::process::Command, timeout: Duration) -> Result<std::process::Output> {
    tokio::time::timeout(timeout,command.output()).await
        .map_err(|_| invalid("本地成片处理超时"))?.map_err(Into::into)
}

async fn render(binary: &Path, clips: &[Clip], (width,height): (u32,u32), dir: &Path) -> Result<PathBuf> {
    let filter = format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=24");
    let mut manifest = String::new();
    for (index,clip) in clips.iter().enumerate() {
        let audio = if clip.still { false } else {
            let mut probe = ffmpeg(binary);
            probe.arg("-i").arg(&clip.path);
            let result = process(probe,Duration::from_secs(15)).await?;
            String::from_utf8_lossy(&result.stderr).lines().any(|line| line.contains("Stream #") && line.contains("Audio:"))
        };
        let name = format!("part-{index}.mp4");
        let mut cmd = ffmpeg(binary);
        cmd.args(["-v","error","-y","-filter_threads","2","-progress","pipe:1","-nostats"]);
        if clip.still { cmd.args(["-loop","1"]); }
        else { cmd.arg("-ss").arg(clip.start.to_string()); }
        cmd.arg("-i").arg(&clip.path);
        if !audio { cmd.args(["-f","lavfi","-i","anullsrc=r=48000:cl=stereo"]); }
        cmd.args(["-map","0:v:0","-map",if audio {"0:a:0"} else {"1:a:0"},"-vf",&filter,
            "-af",&format!("volume={},apad",clip.volume),"-t",&clip.duration.to_string(),"-shortest",
            "-c:v","libx264","-preset","veryfast","-pix_fmt","yuv420p","-threads","2","-c:a","aac","-ar","48000","-ac","2"])
            .arg(dir.join(&name));
        let result = process(cmd,Duration::from_secs(300)).await?;
        if !result.status.success() {
            return Err(invalid(&format!("无法渲染第 {} 个片段，请核对素材、入点和 FFmpeg 编码支持",index+1)));
        }
        let rendered = String::from_utf8_lossy(&result.stdout).lines().filter_map(|line|
            line.strip_prefix("out_time_us=").and_then(|value| value.parse::<u64>().ok())).max().unwrap_or_default();
        if rendered as f64 / 1_000_000.0 + 0.12 < clip.duration {
            return Err(invalid(&format!("第 {} 个片段短于设置时长，请调整入点或裁切时长",index+1)));
        }
        manifest.push_str(&format!("file '{name}'\n"));
    }
    fs::write(dir.join("concat.txt"),manifest)?;
    let result = dir.join("final.mp4");
    let mut cmd = ffmpeg(binary);
    cmd.current_dir(dir).args(["-v","error","-y","-f","concat","-safe","1","-i","concat.txt","-c","copy","-movflags","+faststart","final.mp4"]);
    if !process(cmd,Duration::from_secs(120)).await?.status.success() || fs::metadata(&result)?.len() < 1024 {
        return Err(invalid("无法合成成片"));
    }
    Ok(result)
}

fn archive(app: &App, project_id: &str, id: &str, stamp: &str, previous: &Value, movie: &Path, total: f64, size: (u32,u32)) -> Result<Value> {
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    let (file,mut project) = studio_mcp::project_by_id(app,project_id)?;
    let dir = file.parent().ok_or_else(|| invalid("项目目录无效"))?;
    let manifest = scoped_path(dir,&dir.join("assets/manifest.json"))?;
    let mut assets: Vec<Value> = match fs::read(&manifest) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| invalid("素材清单无效"))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => vec![],
        Err(error) => return Err(error.into()),
    };
    let asset_id = format!("final-{id}");
    let rel = format!("assets/videos/{asset_id}.mp4");
    let target = scoped_path(dir,&dir.join(&rel))?;
    if target.exists() || assets.iter().any(|item| item["id"] == asset_id) { return Err(invalid("成片素材 ID 已存在")); }
    let mut staged = tempfile::NamedTempFile::new_in(target.parent().ok_or_else(|| invalid("素材目录无效"))?)?;
    io::copy(&mut fs::File::open(movie)?,staged.as_file_mut())?;
    staged.as_file().sync_all()?;
    staged.persist_noclobber(&target).map_err(|error|error.error)?;
    assets.push(json!({"id":asset_id,"kind":"video","filename":format!("{asset_id}.mp4"),"mimeType":"video/mp4",
        "size":fs::metadata(&target)?.len(),"source":"ai-generated","fileRelPath":rel,"projectId":project_id,
        "duration":total,"width":size.0,"height":size.1,"fps":24,"hasAudio":true,"createdAt":video_queue::now()}));
    if let Err(error) = atomic_save(&manifest,&serde_json::to_vec_pretty(&assets).map_err(|_| invalid("无法保存素材清单"))?) {
        let _ = fs::remove_file(&target);
        return Err(error);
    }
    let mut selected = timeline_stamp(&project) == stamp && project["finalVideoAssetId"] == *previous;
    if selected {
        project["finalVideoAssetId"] = json!(asset_id);
        project["finalVideoExport"] = json!({"assetId":asset_id,"timelineStamp":stamp});
        project["updatedAt"] = json!(video_queue::now().max(project["updatedAt"].as_u64().unwrap_or_default().saturating_add(1)));
        selected = atomic_save(&file,&serde_json::to_vec_pretty(&project).map_err(|_| invalid("无法保存项目"))?).is_ok();
    }
    Ok(json!({"assetId":asset_id,"finalVideoAssetId":selected.then_some(asset_id),"path":target,"selected":selected}))
}

fn response(mut job: Value, current: Option<&Value>, reused: bool) -> Value {
    let matches = current.and_then(|project| {
        if let Some(stamp) = job["timelineStamp"].as_str().filter(|stamp| stamp.starts_with("v1-")) {
            return Some(timeline_stamp(project) == stamp);
        }
        let asset_id = job["output"]["assetId"].as_str()?;
        if project["finalVideoAssetId"] != asset_id { return None; }
        stale(project).map(|stale| !stale)
    });
    if let Some(fields) = job.as_object_mut() { fields.remove("timelineStamp"); }
    job["matchesCurrentTimeline"] = json!(matches);
    if reused && matches == Some(false) {
        job["reuseNotice"] = json!("此请求 ID 对应旧时间线。导出当前时间线时请使用新的请求 ID。");
    }
    json!({"job":job,"reused":reused})
}

fn latest_job(app: &App, project_id: &str) -> Result<Option<Value>> {
    let mut latest = None;
    for entry in fs::read_dir(state_directory(&app.root)?)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || path.extension().is_none_or(|ext| ext != "json") { continue; }
        let Ok(job) = studio_mcp::read_json(&path) else { continue; };
        let Some(id) = job["requestId"].as_str().filter(|_| job["projectId"] == project_id) else { continue; };
        if path != kv_path(app,&format!("final-export:{}",json!([project_id,id])))? { continue; }
        let key = (entry.metadata()?.modified()?, id.to_owned());
        if latest.as_ref().is_none_or(|(previous,_)| &key > previous) { latest = Some((key,job)); }
    }
    Ok(latest.map(|(_,job)| job))
}

fn stored_response(app: &App, active: Option<&(String,String)>, project_id: &str, mut job: Value) -> Value {
    let tag = (project_id.to_owned(),format!("export:{}",job["requestId"].as_str().unwrap_or_default()));
    if job["status"] == "running" && active != Some(&tag) {
        job["status"] = json!("interrupted");
        job["error"] = json!("服务已退出，使用新请求 ID 重新导出");
    }
    let current = studio_mcp::project_by_id(app,project_id).ok().map(|(_,project)| project);
    response(job,current.as_ref(),true)
}

pub(super) fn call(app: &App, name: &str, input: &Value) -> Result<Value> {
    let project_id = input["projectId"].as_str().filter(|id| !id.is_empty()).ok_or_else(|| invalid("缺少项目 ID"))?;
    let id = input.get("requestId").map(|value| {
        value.as_str().filter(|id| id.len()==36 && id.bytes().enumerate()
            .all(|(i,ch)| if matches!(i,8|13|18|23) { ch==b'-' } else { ch.is_ascii_hexdigit() }))
            .map(str::to_ascii_lowercase).ok_or_else(|| invalid("请求 ID 必须使用 UUID"))
    }).transpose()?;
    if input["confirmed"] == true && id.is_none() { return Err(invalid("启动导出时须传入 UUID 请求 ID")); }
    let mut active = app.video_active.lock().map_err(|_| invalid("制作队列锁不可用"))?;
    let _guard = app.writes.lock().map_err(|_| invalid("项目写锁不可用"))?;
    if name == "studio_export_status" && id.is_none() {
        studio_mcp::project_by_id(app,project_id)?;
        return Ok(match latest_job(app,project_id)? {
            Some(job) => stored_response(app,active.as_ref(),project_id,job),
            None => json!({"job":null,"reused":false}),
        });
    }
    let path = id.as_ref().map(|id| kv_path(app,&format!("final-export:{}",json!([project_id,id])))).transpose()?;
    if let Some(path) = path.as_ref().filter(|path| path.exists()) {
        return Ok(stored_response(app,active.as_ref(),project_id,studio_mcp::read_json(path)?));
    }
    if name == "studio_export_status" { return Err(studio_mcp::missing("找不到该导出任务")); }
    let (file,project) = studio_mcp::project_by_id(app,project_id)?;
    let revision = project["updatedAt"].as_u64().ok_or_else(|| invalid("项目修订号无效"))?;
    validate_project_revision(input, revision, input["confirmed"] != true)?;
    if input.get("burnSubtitles").is_some_and(|value| !value.is_boolean()) { return Err(invalid("burnSubtitles 须为布尔值")); }
    if input["burnSubtitles"] == true { return Err(invalid("服务端基础导出不烧录字幕，请使用界面导出")); }
    let (clips,size,total) = plan(app,&file,&project,&[])?;
    let stamp = timeline_stamp(&project);
    let previous = project["finalVideoAssetId"].clone();
    let mut job = json!({"requestId":id,"projectId":project_id,"status":"prepared","updatedAt":revision,
        "clipCount":clips.len(),"duration":total,"width":size.0,"height":size.1,"burnSubtitles":false,"timelineStamp":stamp});
    let binary = super::ffmpeg_binary().ok().filter(|path| path.is_file())
        .ok_or_else(|| invalid("请配置本地 DRAMLO_FFMPEG_BIN 后再导出"))?;
    if input["confirmed"] != true { return Ok(response(job,Some(&project),false)); }
    let (Some(id), Some(path)) = (id, path) else { return Err(invalid("启动导出时须传入 UUID 请求 ID")); };
    let tag = (project_id.to_owned(),format!("export:{id}"));
    if active.is_some() { return Err(Failure::new(StatusCode::LOCKED,"本机已有制作任务运行".into())); }
    let directory = scoped_path(&app.root,&file.parent().ok_or_else(|| invalid("项目目录无效"))?.join("assets/videos"))?;
    fs::create_dir_all(&directory)?;
    let temp = tempfile::tempdir_in(directory)?;
    job["status"] = json!("running");
    atomic_save(&path,job.to_string().as_bytes())?;
    let started = response(job.clone(),Some(&project),false);
    *active = Some(tag);
    let app = app.clone();
    let project_id = project_id.to_owned();
    tokio::spawn(async move {
        let result = render(&binary,&clips,size,temp.path()).await
            .and_then(|movie| archive(&app,&project_id,&id,&stamp,&previous,&movie,total,size));
        match result {
            Ok(output) => { job["status"] = json!("succeeded"); job["output"] = output; }
            Err(error) => { job["status"] = json!("failed"); job["error"] = json!(error.1); }
        }
        if let Ok(mut active) = app.video_active.lock() {
            if let Ok(_guard) = app.writes.lock() { let _ = atomic_save(&path,job.to_string().as_bytes()); }
            *active = None;
        }
    });
    Ok(started)
}
