use super::*;

pub(super) fn configured() -> Option<String> {
    env::var("DRAMLO_MCP_BRIDGE_TOKEN").ok().filter(|value| value.len() >= 32)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, io::Write, ptr::{null, null_mut}};

    #[repr(C)]
    struct Blob { len: u32, data: *mut u8 }

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(input: *const Blob, description: *const u16, entropy: *const Blob,
            reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
        fn CryptUnprotectData(input: *const Blob, description: *mut *mut u16, entropy: *const Blob,
            reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" { fn LocalFree(memory: *mut c_void) -> *mut c_void; }

    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, output: *mut u8, length: u32, flags: u32) -> i32;
    }

    fn crypt(data: &mut [u8], protect: bool) -> Result<Vec<u8>> {
        let input = Blob { len: data.len() as u32, data: data.as_mut_ptr() };
        let mut output = Blob { len: 0, data: null_mut() };
        // PowerShell ConvertFrom-SecureString uses current-user DPAPI on UTF-16LE bytes.
        // CRYPTPROTECT_UI_FORBIDDEN prevents credential prompts in an stdio client.
        unsafe {
            let ok = if protect {
                CryptProtectData(&input, null(), null(), null_mut(), null_mut(), 1, &mut output)
            } else {
                CryptUnprotectData(&input, null_mut(), null(), null_mut(), null_mut(), 1, &mut output)
            };
            if ok == 0 { return Err(invalid("无法用当前 Windows 用户解密或保存桌面令牌")); }
            let bytes = if output.len == 0 || output.data.is_null() {
                Vec::new()
            } else { std::slice::from_raw_parts(output.data, output.len as usize).to_vec() };
            LocalFree(output.data.cast());
            Ok(bytes)
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn read(path: &Path) -> Result<Option<String>> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        use std::io::Read;
        let mut text = String::new();
        (&mut file).take(8193).read_to_string(&mut text)?;
        let text = text.trim().trim_start_matches('\u{feff}').as_bytes();
        if text.is_empty() || text.len() > 8192 || text.len() % 2 != 0 || !text.iter().all(u8::is_ascii_hexdigit) {
            return Err(invalid("桌面令牌文件格式无效"));
        }
        let mut data = text.chunks_exact(2).map(|pair|
            u8::from_str_radix(std::str::from_utf8(pair).expect("hex ASCII"), 16).expect("validated hex")
        ).collect::<Vec<_>>();
        let bytes = crypt(&mut data, false)?;
        if bytes.len() % 2 != 0 { return Err(invalid("桌面令牌编码无效")); }
        let units = bytes.chunks_exact(2).map(|v| u16::from_le_bytes([v[0], v[1]])).collect::<Vec<_>>();
        let token = String::from_utf16(&units).map_err(|_| invalid("桌面令牌编码无效"))?;
        if token.len() < 32 || !token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(invalid("桌面令牌格式无效"));
        }
        Ok(Some(token))
    }

    pub(super) fn load(create: bool) -> Result<Option<String>> {
        let exe = env::current_exe()?;
        let directory = exe.parent().ok_or_else(|| invalid("找不到桌面启动器目录"))?;
        let path = directory.join("launcher-token.dpapi");
        if let Some(token) = read(&path)? { return Ok(Some(token)); }
        if !create { return Ok(None); }
        let mut random = [0u8; 32];
        if unsafe { BCryptGenRandom(null_mut(), random.as_mut_ptr(), 32, 2) } != 0 {
            return Err(invalid("无法生成桌面令牌"));
        }
        let token = hex(&random);
        let mut plain = token.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        temporary.write_all(hex(&crypt(&mut plain, true)?).as_bytes())?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => Ok(Some(token)),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => read(&path),
            Err(error) => Err(error.error.into()),
        }
    }
}

pub(super) fn launcher(create: bool) -> Result<Option<String>> {
    #[cfg(windows)]
    { windows::load(create) }
    #[cfg(not(windows))]
    { let _ = create; Err(invalid("桌面 DPAPI 令牌仅支持 Windows")) }
}
