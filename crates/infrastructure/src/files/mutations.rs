use super::*;
use runtime_domain::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    volume: u64,
    file: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Mutation {
    Directory {
        path: String,
        staging: String,
        identity: Identity,
    },
    Move {
        source: String,
        destination: String,
        identity: Identity,
        snapshot: Box<crate::storage::SnapshotDto>,
    },
}
fn guarded_path(fs: &WorkspaceFiles, path: &str) -> Result<PathBuf> {
    let relative = fs.validate(path)?;
    if relative == Path::new(".")
        || relative
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(".gitignore"))
    {
        return Err(PortError::Policy(
            "workspace root and ignore policy cannot be mutated".into(),
        ));
    }
    Ok(relative)
}
fn parent_name(fs: &WorkspaceFiles, relative: &Path) -> Result<(Dir, std::ffi::OsString)> {
    let parent = relative
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = relative
        .file_name()
        .ok_or_else(|| PortError::Policy("missing filename".into()))?
        .to_owned();
    Ok((fs.dir.open_dir(parent).map_err(io_error)?, name))
}
fn identity(file: &std::fs::File) -> Result<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata().map_err(io_error)?;
        Ok(Identity {
            volume: m.dev(),
            file: m.ino(),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(Identity {
            volume: info.dwVolumeSerialNumber as u64,
            file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        })
    }
}
fn identity_at(fs: &WorkspaceFiles, path: &Path, directory: bool) -> Result<Identity> {
    if directory {
        identity(&fs.dir.open_dir(path).map_err(io_error)?.into_std_file())
    } else {
        identity(&fs.dir.open(path).map_err(io_error)?.into_std())
    }
}
#[cfg(windows)]
fn mutation_handle(
    parent: &Dir,
    name: &std::ffi::OsStr,
    expected: &Identity,
) -> Result<std::fs::File> {
    use std::os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    };
    #[repr(C)]
    struct UnicodeString {
        len: u16,
        max_len: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        len: u32,
        root: *mut std::ffi::c_void,
        name: *mut UnicodeString,
        attributes: u32,
        descriptor: *mut std::ffi::c_void,
        qos: *mut std::ffi::c_void,
    }
    #[repr(C)]
    struct IoStatus {
        status: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            handle: *mut *mut std::ffi::c_void,
            access: u32,
            attributes: *mut ObjectAttributes,
            status: *mut IoStatus,
            size: *const i64,
            file_attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            ea: *mut std::ffi::c_void,
            ea_len: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let bytes =
        u16::try_from(wide.len() * 2).map_err(|_| PortError::Policy("filename too long".into()))?;
    let mut unicode = UnicodeString {
        len: bytes,
        max_len: bytes,
        buffer: wide.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        len: std::mem::size_of::<ObjectAttributes>() as u32,
        root: parent.as_raw_handle(),
        name: &mut unicode,
        attributes: 0x40,
        descriptor: std::ptr::null_mut(),
        qos: std::ptr::null_mut(),
    };
    let mut status = IoStatus {
        status: 0,
        information: 0,
    };
    let mut handle = std::ptr::null_mut();
    // Open the final component directly relative to the pinned directory. cap-std's
    // directory traversal pins it without SHARE_DELETE, conflicting with DELETE
    // access if open_with is used for the same directory. No absolute-path fallback.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            0x00110080,
            &mut attributes,
            &mut status,
            std::ptr::null(),
            0,
            3,
            1,
            0x00200020,
            std::ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(io_error(std::io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(result)
        }
            as i32)));
    }
    let file = unsafe { std::fs::File::from_raw_handle(handle) };
    use std::os::windows::fs::MetadataExt;
    if file.metadata().map_err(io_error)?.file_attributes() & 0x400 != 0
        || identity(&file)? != *expected
    {
        return Err(PortError::Conflict);
    }
    Ok(file)
}

fn rename_no_replace(
    fs: &WorkspaceFiles,
    source: &Path,
    destination: &Path,
    expected: &Identity,
) -> Result<()> {
    let (from, name) = parent_name(fs, source)?;
    let (to, target) = parent_name(fs, destination)?;
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::FILE_RENAME_INFO;
        #[repr(C)]
        struct IoStatus {
            status: usize,
            information: usize,
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtSetInformationFile(
                handle: *mut std::ffi::c_void,
                status: *mut IoStatus,
                info: *const std::ffi::c_void,
                size: u32,
                class: u32,
            ) -> i32;
            fn RtlNtStatusToDosError(status: i32) -> u32;
        }
        let source = mutation_handle(&from, &name, expected)?;
        let wide: Vec<u16> = target.encode_wide().collect();
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let size = offset + wide.len() * 2;
        let mut storage = vec![0u64; size.div_ceil(8)];
        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = to.as_raw_handle();
            (*info).FileNameLength = (wide.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                wide.as_ptr(),
                std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                wide.len(),
            );
            let mut status = IoStatus {
                status: 0,
                information: 0,
            };
            let result = NtSetInformationFile(
                source.as_raw_handle(),
                &mut status,
                info.cast(),
                size as u32,
                10,
            );
            if result < 0 {
                return Err(PortError::Tool(format!(
                    "rename handle: {}",
                    std::io::Error::from_raw_os_error(RtlNtStatusToDosError(result) as i32)
                )));
            }
        }
    }
    #[cfg(unix)]
    {
        if &identity_at(fs, source, false).or_else(|_| identity_at(fs, source, true))? != expected {
            return Err(PortError::Conflict);
        }
        use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
        let a = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| PortError::Policy("invalid path".into()))?;
        let b = std::ffi::CString::new(target.as_bytes())
            .map_err(|_| PortError::Policy("invalid path".into()))?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                from.as_raw_fd(),
                a.as_ptr(),
                to.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                from.as_raw_fd(),
                a.as_ptr(),
                to.as_raw_fd(),
                b.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        return Err(PortError::Policy(
            "atomic no-replace rename unsupported on this OS".into(),
        ));
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if result != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        from.open(".")
            .map_err(io_error)?
            .sync_all()
            .map_err(io_error)?;
        to.open(".")
            .map_err(io_error)?
            .sync_all()
            .map_err(io_error)?;
    }
    Ok(())
}
impl WorkspaceFiles {
    pub(crate) fn prepare_directory(&self, path: &str, operation: OperationId) -> Result<Mutation> {
        let relative = guarded_path(self, path)?;
        match self.dir.symlink_metadata(&relative) {
            Ok(_) => return Err(PortError::Conflict),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
        let (parent, _) = parent_name(self, &relative)?;
        let temporary = format!(".rdc-{operation}.directory");
        parent.create_dir(&temporary).map_err(io_error)?;
        let staging = relative.with_file_name(&temporary);
        let identity = identity_at(self, &staging, true)?;
        #[cfg(unix)]
        parent
            .open(".")
            .map_err(io_error)?
            .sync_all()
            .map_err(io_error)?;
        Ok(Mutation::Directory {
            path: path.into(),
            staging: staging.to_string_lossy().into_owned(),
            identity,
        })
    }
    pub(crate) fn prepare_move(&self, source: &str, destination: &str) -> Result<Mutation> {
        let from = guarded_path(self, source)?;
        let to = guarded_path(self, destination)?;
        if from == to {
            return Err(PortError::Conflict);
        }
        parent_name(self, &from)?;
        parent_name(self, &to)?;
        if self.snapshot_sync(destination)?.content.is_some() {
            return Err(PortError::Conflict);
        }
        let snapshot = self.snapshot_sync(source)?;
        if snapshot.content.is_none() {
            return Err(PortError::NotFound);
        }
        let identity = identity_at(self, &from, false)?;
        Ok(Mutation::Move {
            source: source.into(),
            destination: destination.into(),
            identity,
            snapshot: Box::new(crate::storage::SnapshotDto::from(snapshot)),
        })
    }
    pub(crate) fn apply_mutation(&self, mutation: &Mutation) -> Result<()> {
        match mutation {
            Mutation::Directory {
                path,
                staging,
                identity,
            } => {
                let target = guarded_path(self, path)?;
                let stage = Path::new(staging);
                if &identity_at(self, stage, true)? != identity {
                    return Err(PortError::Conflict);
                }
                rename_no_replace(self, stage, &target, identity)?;
                if &identity_at(self, &target, true)? != identity {
                    return Err(PortError::Conflict);
                }
                Ok(())
            }
            Mutation::Move {
                source,
                destination,
                identity,
                snapshot,
            } => {
                let from = guarded_path(self, source)?;
                let to = guarded_path(self, destination)?;
                if &identity_at(self, &from, false)? != identity {
                    return Err(PortError::Conflict);
                }
                self.check_snapshot_sync(&snapshot.as_ref().clone().into())?;
                rename_no_replace(self, &from, &to, identity)?;
                if &identity_at(self, &to, false)? != identity {
                    return Err(PortError::Conflict);
                }
                let mut expected: FileSnapshot = snapshot.as_ref().clone().into();
                expected.path = destination.clone();
                self.check_snapshot_sync(&expected)
            }
        }
    }
    pub(crate) fn rollback_mutation(&self, mutation: &Mutation) -> Result<()> {
        match mutation {
            Mutation::Directory {
                path,
                staging,
                identity,
            } => {
                let target = guarded_path(self, path)?;
                for path in [&target, Path::new(staging)] {
                    match identity_at(self, path, true) {
                        Ok(actual) if &actual == identity => {
                            let directory = self.dir.open_dir(path).map_err(io_error)?;
                            if directory.entries().map_err(io_error)?.next().is_some() {
                                return Err(PortError::Conflict);
                            }
                            drop(directory);
                            let (parent, name) = parent_name(self, path)?;
                            #[cfg(windows)]
                            {
                                use std::os::windows::io::AsRawHandle;
                                use windows_sys::Win32::Storage::FileSystem::{
                                    FILE_DISPOSITION_INFO, FileDispositionInfo,
                                    SetFileInformationByHandle,
                                };
                                let handle = mutation_handle(&parent, &name, identity)?;
                                let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
                                if unsafe {
                                    SetFileInformationByHandle(
                                        handle.as_raw_handle(),
                                        FileDispositionInfo,
                                        (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                                        std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                                    )
                                } == 0
                                {
                                    return Err(io_error(std::io::Error::last_os_error()));
                                }
                            }
                            #[cfg(not(windows))]
                            parent.remove_dir(name).map_err(io_error)?;
                            #[cfg(unix)]
                            parent
                                .open(".")
                                .map_err(io_error)?
                                .sync_all()
                                .map_err(io_error)?;
                        }
                        Err(PortError::NotFound) => {}
                        _ => return Err(PortError::Conflict),
                    }
                }
                Ok(())
            }
            Mutation::Move {
                source,
                destination,
                identity,
                snapshot,
            } => {
                let from = guarded_path(self, source)?;
                let to = guarded_path(self, destination)?;
                match (
                    identity_at(self, &from, false),
                    identity_at(self, &to, false),
                ) {
                    (Ok(actual), Err(PortError::NotFound)) if &actual == identity => {
                        self.check_snapshot_sync(&snapshot.as_ref().clone().into())
                    }
                    (Err(PortError::NotFound), Ok(actual)) if &actual == identity => {
                        let mut expected: FileSnapshot = snapshot.as_ref().clone().into();
                        expected.path = destination.clone();
                        self.check_snapshot_sync(&expected)?;
                        rename_no_replace(self, &to, &from, identity)
                    }
                    _ => Err(PortError::Conflict),
                }
            }
        }
    }
}
