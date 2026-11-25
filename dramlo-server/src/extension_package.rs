use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::{Cursor, Read, Write};

const ARCHIVE_LIMIT: usize = 128 * 1024;
const MANIFEST_LIMIT: usize = 64 * 1024;

pub(super) fn read(input: &Value) -> Result<Value> {
    let encoded = input["packageBase64"].as_str().filter(|value| value.len() <= ARCHIVE_LIMIT.div_ceil(3) * 4)
        .ok_or_else(|| invalid("扩展 ZIP 为空或超过 128 KB"))?;
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid("扩展 ZIP 编码无效"))?;
    if bytes.len() > ARCHIVE_LIMIT { return Err(invalid("扩展 ZIP 超过 128 KB")); }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid("无法读取扩展 ZIP"))?;
    if archive.len() != 1 { return Err(invalid("扩展 ZIP 只接受根目录的 extension.json")); }
    let mut entry = archive.by_index(0).map_err(|_| invalid("无法读取扩展清单"))?;
    if entry.name() != "extension.json" || !entry.is_file() || entry.is_symlink() || entry.size() > MANIFEST_LIMIT as u64 {
        return Err(invalid("扩展 ZIP 须包含 64 KB 以内的根目录 extension.json"));
    }
    let mut contents = Vec::new();
    (&mut entry).take((MANIFEST_LIMIT + 1) as u64).read_to_end(&mut contents)?;
    if contents.len() > MANIFEST_LIMIT { return Err(invalid("扩展清单过大")); }
    serde_json::from_slice(&contents).map_err(|_| invalid("扩展清单 JSON 无效"))
}

pub(super) fn export(manifest: &Value) -> Result<Value> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("extension.json", zip::write::SimpleFileOptions::default())
        .map_err(|_| invalid("无法创建扩展 ZIP"))?;
    writer.write_all(&serde_json::to_vec_pretty(manifest).map_err(|_| invalid("无法序列化扩展清单"))?)?;
    let data = writer.finish().map_err(|_| invalid("无法完成扩展 ZIP"))?.into_inner();
    Ok(json!({ "packageBase64": STANDARD.encode(data) }))
}
