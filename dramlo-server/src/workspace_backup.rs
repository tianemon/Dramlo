use super::*;
use std::{collections::BTreeSet, io::{Read, Write}};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const LIMIT: u64 = 64 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const MARKER: &str = "dramlo-backup.json";

fn project_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c,b'-'|b'_'))
}

fn safe_component(part: &str) -> bool {
    let base = part.split('.').next().unwrap_or_default().to_ascii_uppercase();
    !part.is_empty() && part != "." && part != ".." && !part.ends_with([' ','.'])
        && !part.chars().any(|c| c.is_control() || matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && !matches!(base.as_str(),"CON"|"PRN"|"AUX"|"NUL"|"CONIN$"|"CONOUT$")
        && !["COM","LPT"].iter().any(|prefix| base.strip_prefix(prefix)
            .is_some_and(|suffix| matches!(suffix,"0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|"¹"|"²"|"³")))
}

fn file_allowed(path: &Path) -> bool {
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or_default().to_ascii_lowercase();
    path == Path::new("project.json") || path == Path::new(project_authoring::PROJECT_INSTRUCTIONS) ||
        path == Path::new("assets/manifest.json") ||
        ((path.starts_with("assets") || path.starts_with("thumbs")) &&
            ["png","jpg","jpeg","webp","gif","mp4","webm","mov","wav","mp3","m4a","ogg","flac","bin"].contains(&ext.as_str())) ||
        (path.starts_with("docs") && ["md","txt","srt"].contains(&ext.as_str()))
}

fn scrub_metadata(value: &mut Value) {
    match value {
        Value::Object(entries) => {
            entries.retain(|key,_| {
                let key = key.to_ascii_lowercase().replace(['_', '-'], "");
                !matches!(key.as_str(),"folderpath"|"apikey"|"token"|"authorization"|"credential"|"secret"|"password"|"accesskey"|"refreshtoken"|"accesstoken"|"headers")
            });
            for value in entries.values_mut() { scrub_metadata(value); }
        }
        Value::Array(entries) => entries.iter_mut().for_each(scrub_metadata),
        _ => {}
    }
}

fn validate_media_manifest(folder: &Path, manifest: &Value) -> Result<()> {
    for item in manifest.as_array().ok_or_else(|| invalid("备份素材清单无效"))? {
        let relative = Path::new(item["fileRelPath"].as_str().ok_or_else(|| invalid("备份素材缺少路径"))?);
        if !relative.starts_with("assets") || relative.components().count() < 3 ||
            !file_allowed(relative) || !scoped_path(folder,relative)?.is_file() {
            return Err(invalid("备份缺少清单中的素材文件，请检查项目资源库"));
        }
    }
    Ok(())
}

fn write_files(writer: &mut ZipWriter<fs::File>, folder: &Path, relative: &Path, id: &str, quota: &mut (usize,u64)) -> Result<()> {
    for entry in fs::read_dir(folder.join(relative))? {
        let entry = entry?;
        let next = relative.join(entry.file_name());
        let ty = entry.file_type()?;
        if ty.is_dir() && ["assets","thumbs","docs"].iter().any(|name| next.starts_with(name)) {
            if next.components().count() > 24 { return Err(invalid("项目目录层级过深")); }
            write_files(writer,folder,&next,id,quota)?;
        }
        if !ty.is_file() || !file_allowed(&next) { continue; }
        let source = scoped_path(folder,&next)?;
        let size = fs::metadata(&source)?.len();
        quota.0 += 1;
        quota.1 = quota.1.saturating_add(size);
        if quota.0 > MAX_FILES || quota.1 > LIMIT { return Err(invalid("备份文件超过数量或大小限制")); }
        let name = next.to_str().ok_or_else(|| invalid("文件名无法保存"))?.replace('\\',"/");
        let options = SimpleFileOptions::default().large_file(size >= u64::from(u32::MAX));
        writer.start_file(format!("projects/{id}/{name}"), options).map_err(|_|invalid("无法写入备份"))?;
        if next == Path::new("project.json") || next == Path::new("assets/manifest.json") {
            let mut data = studio_mcp::read_json(&source)?;
            if next == Path::new("assets/manifest.json") { validate_media_manifest(folder,&data)?; }
            scrub_metadata(&mut data);
            writer.write_all(data.to_string().as_bytes())?;
        } else { io::copy(&mut fs::File::open(source)?,writer)?; }
    }
    Ok(())
}

fn backup(app: App) -> Result<fs::File> {
    let _write = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
    let mut writer = ZipWriter::new(tempfile::tempfile_in(state_directory(&app.root)?)?);
    writer.start_file(MARKER,SimpleFileOptions::default()).map_err(|_| invalid("无法创建备份"))?;
    writer.write_all(b"{\"version\":1}")?;
    let mut quota = (0,0u64);
    let mut seen = BTreeSet::new();
    for entry in fs::read_dir(&app.root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() { continue; }
        let folder = entry.path();
        let file = folder.join("project.json");
        if !file.is_file() { continue; }
        let project = studio_mcp::read_json(&scoped_path(&app.root,&file)?)?;
        let id = project["id"].as_str().filter(|id| project_id(id)).ok_or_else(|| invalid("项目 ID 无效"))?;
        if !seen.insert(id.to_ascii_lowercase()) { return Err(invalid("项目 ID 重复，无法打包")); }
        write_files(&mut writer,&folder,Path::new(""),id,&mut quota)?;
    }
    let mut file = writer.finish().map_err(|_| invalid("无法完成备份"))?;
    use std::io::Seek;
    file.rewind()?;
    Ok(file)
}

fn restore(app: App, file: fs::File) -> Result<Value> {
    let mut zip = ZipArchive::new(file).map_err(|_|invalid("请选择 Dramlo 项目 ZIP 备份"))?;
    if zip.len() > MAX_FILES + 1 { return Err(invalid("备份文件过多")); }
    {
        let mut marker = zip.by_name(MARKER).map_err(|_|invalid("这不是 Dramlo 项目备份"))?;
        let mut version = String::new();
        Read::by_ref(&mut marker).take(512).read_to_string(&mut version)?;
        if serde_json::from_str::<Value>(&version).ok().is_none_or(|v| v["version"] != 1) {
            return Err(invalid("不支持此备份版本"));
        }
    }
    let staging = tempfile::Builder::new().prefix("import-projects-").tempdir_in(state_directory(&app.root)?)?;
    let mut project_ids = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut item = zip.by_index(index).map_err(|_|invalid("无法读取备份条目"))?;
        if item.name() == MARKER { continue; }
        let name = item.name().to_owned();
        let safe = name.split('/').all(safe_component);
        if !safe || name.len()>2048 || item.is_dir() || item.is_symlink() || !seen.insert(name.to_ascii_lowercase()) {
            return Err(invalid("备份中有非法路径或重复文件"));
        }
        let mut parts = name.splitn(3,'/');
        let root = parts.next();
        let id = parts.next().filter(|id| project_id(id)).ok_or_else(|| invalid("备份项目 ID 无效"))?;
        let relative = Path::new(parts.next().ok_or_else(|| invalid("备份路径无效"))?);
        if root != Some("projects") || !file_allowed(relative) { return Err(invalid("备份包含不支持的文件")); }
        total = total.saturating_add(item.size());
        if total > LIMIT { return Err(invalid("解包内容超出上限")); }
        project_ids.insert(id.to_owned());
        let output = staging.path().join(id).join(relative);
        fs::create_dir_all(output.parent().ok_or_else(||invalid("缺少导入目录"))?)?;
        let mut sink = fs::OpenOptions::new().create_new(true).write(true).open(output)?;
        let size = item.size();
        if io::copy(&mut Read::by_ref(&mut item).take(size.saturating_add(1)),&mut sink)? != size {
            return Err(invalid("备份条目长度有误"));
        }
    }
    let _write = app.writes.lock().map_err(|_| invalid("写锁不可用"))?;
    let mut pending = Vec::new();
    for id in project_ids {
        if studio_mcp::project_by_id(&app,&id).is_ok() { return Err(invalid(&format!("已有同 ID 项目 {id}，没有覆盖"))); }
        let original = staging.path().join(&id);
        let target = app.root.join(format!("project__{id}"));
        if target.exists() { return Err(invalid("目标项目目录已存在")); }
        let project_file = original.join("project.json");
        let mut project = studio_mcp::read_json(&project_file)?;
        if project["id"] != id {
            return Err(invalid("备份项目数据无效"));
        }
        studio_mcp::validate_project_document(&project)?;
        scrub_metadata(&mut project);
        project["folderPath"] = json!(target);
        let manifest_file = original.join("assets/manifest.json");
        if manifest_file.is_file() {
            let mut manifest = studio_mcp::read_json(&manifest_file)?;
            validate_media_manifest(&original,&manifest)?;
            scrub_metadata(&mut manifest);
            atomic_save(&manifest_file,manifest.to_string().as_bytes())?;
        }
        atomic_save(&project_file,project.to_string().as_bytes())?;
        pending.push((original,target));
    }
    for (index,(original,target)) in pending.iter().enumerate() {
        if let Err(error) = fs::rename(original,target) {
            let mut rolled_back = true;
            for (from,to) in pending[..index].iter().rev() {
                if fs::rename(to,from).is_err() { rolled_back = false; }
            }
            if !rolled_back { return Err(invalid("导入失败且未能全部回退，请核对项目列表")); }
            return Err(error.into());
        }
    }
    Ok(json!({"imported":pending.len()}))
}

pub(super) async fn download(State(app): State<App>) -> Result<Response> {
    let file = tokio::task::spawn_blocking(move || backup(app)).await.map_err(|_| invalid("备份任务中断"))??;
    let length = file.metadata()?.len();
    let stream = futures_util::stream::try_unfold(tokio::fs::File::from_std(file), |mut file| async move {
        let mut buffer = vec![0u8; 128 * 1024];
        let count = file.read(&mut buffer).await?;
        buffer.truncate(count);
        Ok::<_,io::Error>((count > 0).then_some((Bytes::from(buffer),file)))
    });
    Ok(([(header::CONTENT_TYPE,"application/zip".into()),
        (header::CONTENT_DISPOSITION,"attachment; filename=studio-projects.zip".into()),
        (header::CONTENT_LENGTH,length.to_string())], Body::from_stream(stream)).into_response())
}

pub(super) async fn upload(State(app): State<App>, request: Request) -> Result<Json<Value>> {
    use futures_util::StreamExt;
    let mut chunks = request.into_body().into_data_stream();
    let mut file = tokio::fs::File::from_std(tempfile::tempfile_in(state_directory(&app.root)?)?);
    let mut total = 0u64;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| invalid("读取上传文件失败"))?;
        total = total.saturating_add(chunk.len() as u64);
        if total > LIMIT { return Err(invalid("备份文件过大")); }
        file.write_all(&chunk).await?;
    }
    file.seek(io::SeekFrom::Start(0)).await?;
    let file = file.into_std().await;
    let result = tokio::task::spawn_blocking(move || restore(app,file)).await.map_err(|_| invalid("恢复任务中断"))??;
    Ok(Json(result))
}
