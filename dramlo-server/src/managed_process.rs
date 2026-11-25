use super::*;

#[derive(Clone, Copy, Deserialize, serde::Serialize)]
pub(super) struct Identity {
    pub pid: u32,
    pub created: u64,
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::c_void;

    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime { low: u32, high: u32 }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(process: Handle) -> i32;
        fn GetProcessTimes(process: Handle, created: *mut FileTime, exited: *mut FileTime,
            kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn WaitForSingleObject(process: Handle, millis: u32) -> u32;
        fn TerminateProcess(process: Handle, code: u32) -> i32;
    }

    struct Process(Handle);

    impl Drop for Process {
        fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
    }

    impl Process {
        fn open(pid: u32, terminate: bool) -> io::Result<Option<Self>> {
            const QUERY_AND_WAIT: u32 = 0x1000 | 0x0010_0000;
            let handle = unsafe { OpenProcess(QUERY_AND_WAIT | u32::from(terminate), 0, pid) };
            if !handle.is_null() { return Ok(Some(Self(handle))); }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(87) { return Ok(None); }
            Err(error)
        }

        fn created(&self) -> io::Result<u64> {
            let mut created = FileTime::default();
            let mut exited = FileTime::default();
            let mut kernel = FileTime::default();
            let mut user = FileTime::default();
            if unsafe { GetProcessTimes(self.0, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok((u64::from(created.high) << 32) | u64::from(created.low))
        }

        fn matches(&self, id: Identity) -> io::Result<bool> {
            Ok(self.created()? == id.created && unsafe { WaitForSingleObject(self.0, 0) } == 258)
        }
    }

    pub(super) fn identify(pid: u32) -> io::Result<Identity> {
        let process = Process::open(pid, false)?.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "进程已结束"))?;
        Ok(Identity { pid, created: process.created()? })
    }

    pub(super) fn alive(id: Identity) -> io::Result<bool> {
        match Process::open(id.pid, false)? {
            Some(process) => process.matches(id),
            None => Ok(false),
        }
    }

    pub(super) fn stop(id: Identity) -> io::Result<bool> {
        let Some(process) = Process::open(id.pid, true)? else { return Ok(false); };
        if !process.matches(id)? { return Ok(false); }
        if unsafe { TerminateProcess(process.0, 1) } == 0 { return Err(io::Error::last_os_error()); }
        match unsafe { WaitForSingleObject(process.0, 3000) } {
            0 => Ok(true),
            258 => Err(io::Error::new(io::ErrorKind::TimedOut, "等待 ComfyUI 退出超时")),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

#[cfg(windows)]
pub(super) fn identify(pid: u32) -> io::Result<Option<Identity>> { windows::identify(pid).map(Some) }
#[cfg(not(windows))]
pub(super) fn identify(_: u32) -> io::Result<Option<Identity>> { Ok(None) }

pub(super) fn alive(id: Identity) -> io::Result<bool> {
    #[cfg(windows)] { windows::alive(id) }
    #[cfg(not(windows))] { let _ = id; Ok(false) }
}

pub(super) fn stop(id: Identity) -> io::Result<bool> {
    #[cfg(windows)] { windows::stop(id) }
    #[cfg(not(windows))] { let _ = id; Ok(false) }
}
