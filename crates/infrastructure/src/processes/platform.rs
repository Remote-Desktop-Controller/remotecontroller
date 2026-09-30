use std::io;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::*,
        Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
    },
};
pub(super) struct Job(usize);
impl Job {
    pub(super) fn attach(pid: u32) -> io::Result<Self> {
        // SAFETY: Windows owns these handles; they are closed exactly once.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle as usize);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            let assigned = AssignProcessToJobObject(handle, process);
            let error = io::Error::last_os_error();
            CloseHandle(process);
            if assigned == 0 {
                return Err(error);
            }
            Ok(job)
        }
    }
    pub(super) fn terminate(&self) {
        // SAFETY: handle remains live for this object's lifetime.
        unsafe {
            TerminateJobObject(self.0 as HANDLE, 1);
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: this is the sole owner of the handle.
        unsafe {
            CloseHandle(self.0 as HANDLE);
        }
    }
}
