use std::io;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::*,
        Threading::{
            OpenProcess, OpenThread, PROCESS_SET_QUOTA, PROCESS_TERMINATE, ResumeThread,
            THREAD_SUSPEND_RESUME,
        },
    },
};
pub(super) struct Job(usize);
impl Job {
    pub(super) fn resume_suspended(pid: u32) -> io::Result<()> {
        // The target was created suspended, so its initial thread cannot spawn a
        // descendant before job assignment. Enumerate only this owned child's threads.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut found = Vec::new();
            let mut more = Thread32First(snapshot, &mut entry);
            while more != 0 {
                if entry.th32OwnerProcessID == pid {
                    found.push(entry.th32ThreadID);
                }
                more = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if found.len() != 1 {
                return Err(io::Error::other(
                    "suspended target must have exactly one initial thread",
                ));
            }
            let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, found[0]);
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            let previous = ResumeThread(thread);
            let error = io::Error::last_os_error();
            CloseHandle(thread);
            if previous != 1 {
                return Err(if previous == u32::MAX {
                    error
                } else {
                    io::Error::other("unexpected target suspension count")
                });
            }
            Ok(())
        }
    }
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
