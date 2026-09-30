use std::{io, path::Path};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        SetFileSecurityW,
    },
};

struct Descriptor(*mut std::ffi::c_void);
impl Descriptor {
    fn new() -> io::Result<Self> {
        // Owner Rights and SYSTEM only. Inherited by files and child directories.
        let sddl: Vec<u16> = "D:P(A;OICI;GA;;;OW)(A;OICI;GA;;;SY)\0"
            .encode_utf16()
            .collect();
        let mut ptr = std::ptr::null_mut();
        // SAFETY: sddl is NUL terminated; output pointer lives for the call.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut ptr,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(ptr))
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: pointer allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub(super) fn private_acl(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let desc = Descriptor::new()?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: path and descriptor buffers are live, valid and NUL terminated.
    if unsafe {
        SetFileSecurityW(
            wide.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            desc.0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub(super) fn create_pipe(endpoint: &str, first: bool) -> io::Result<NamedPipeServer> {
    let desc = Descriptor::new()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: desc.0,
        bInheritHandle: 0,
    };
    // SAFETY: attributes and descriptor remain live throughout CreateNamedPipeW.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                endpoint,
                (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
}
