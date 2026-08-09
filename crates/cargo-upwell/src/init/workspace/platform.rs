use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::path::Path;

use super::super::InitError;

pub(super) enum ExchangeError {
    Unsupported(String),
    Io(std::io::Error),
    #[cfg(windows)]
    AfterMutation(std::io::Error),
}

pub(super) fn candidate_path<'a>(displaced: &'a Path, _candidate: &'a Path) -> &'a Path {
    #[cfg(windows)]
    {
        _candidate
    }
    #[cfg(not(windows))]
    {
        displaced
    }
}

pub(super) fn ensure_supported(project: &Path, manifest: &Path) -> Result<(), InitError> {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        let _ = (project, manifest);
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        Err(InitError::UnsupportedWorkspacePublication {
            project: project.to_path_buf(),
            manifest: manifest.to_path_buf(),
            reason: String::from("atomic manifest exchange is not implemented for this platform"),
        })
    }
}

pub(super) fn open_manifest(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
        options.share_mode(FILE_SHARE_READ);
    }
    options.open(path)
}

pub(super) fn open_published(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

pub(super) fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        use std::os::windows::io::FromRawHandle as _;
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers, OPEN_EXISTING,
        };

        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `path` is a live nul-terminated UTF-16 buffer; the returned owned handle is
        // converted to `File` exactly once on success.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `handle` was returned by `CreateFileW` and ownership transfers to `File`.
        let directory = unsafe { File::from_raw_handle(handle as _) };
        // SAFETY: the file owns a valid directory handle accepted by `FlushFileBuffers`.
        let result = unsafe { FlushFileBuffers(handle) };
        if result == 0 {
            let error = std::io::Error::last_os_error();
            drop(directory);
            return Err(error);
        }
        drop(directory);
        let _ = CloseHandle;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        File::open(path)?.sync_all()
    }
}

pub(super) fn exchange(
    _active: &mut File,
    manifest: &Path,
    candidate: &Path,
    _displaced: &Path,
) -> Result<(), ExchangeError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let manifest = std::ffi::CString::new(manifest.as_os_str().as_bytes())
            .map_err(|error| ExchangeError::Io(std::io::Error::other(error)))?;
        let candidate = std::ffi::CString::new(candidate.as_os_str().as_bytes())
            .map_err(|error| ExchangeError::Io(std::io::Error::other(error)))?;
        // SAFETY: both paths are valid nul-terminated strings and the syscall borrows them only.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                manifest.as_ptr(),
                libc::AT_FDCWD,
                candidate.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
            ) {
                return Err(ExchangeError::Unsupported(error.to_string()));
            }
            return Err(ExchangeError::Io(error));
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let manifest = std::ffi::CString::new(manifest.as_os_str().as_bytes())
            .map_err(|error| ExchangeError::Io(std::io::Error::other(error)))?;
        let candidate_c = std::ffi::CString::new(candidate.as_os_str().as_bytes())
            .map_err(|error| ExchangeError::Io(std::io::Error::other(error)))?;
        // SAFETY: both paths are valid nul-terminated strings and the call borrows them only.
        let result =
            unsafe { libc::renamex_np(manifest.as_ptr(), candidate_c.as_ptr(), libc::RENAME_SWAP) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
                return Err(ExchangeError::Unsupported(error.to_string()));
            }
            return Err(ExchangeError::Io(error));
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::io::{Read as _, Seek as _, Write as _};

        let _ = (manifest, _displaced);
        let mut replacement = Vec::new();
        File::open(candidate)
            .and_then(|mut file| file.read_to_end(&mut replacement))
            .map_err(ExchangeError::Io)?;

        _active
            .rewind()
            .and_then(|()| _active.write_all(&replacement))
            .and_then(|()| _active.set_len(replacement.len() as u64))
            .and_then(|()| _active.sync_all())
            .map_err(ExchangeError::AfterMutation)?;

        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (_active, manifest, candidate, _displaced);
        Err(ExchangeError::Unsupported(String::from(
            "atomic manifest exchange is not implemented for this platform",
        )))
    }
}

pub(super) fn verify_publication(
    active: &File,
    manifest: &Path,
    displaced: &Path,
    expected: &[u8],
) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::io::{Read as _, Seek as _};

        let _ = displaced;
        let mut active = active;
        active.rewind()?;
        let mut bytes = Vec::new();
        active.read_to_end(&mut bytes)?;
        if bytes != expected || !path_names_file(active, manifest)? {
            return Err(std::io::Error::other(
                "published manifest bytes or identity did not verify",
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let published = open_published(manifest)?;
        let mut bytes = Vec::new();
        (&published).read_to_end(&mut bytes)?;
        if bytes != expected {
            return Err(std::io::Error::other(
                "published manifest bytes did not verify",
            ));
        }
        if !path_names_file(active, displaced)? {
            return Err(std::io::Error::other(
                "displaced recovery no longer names the original manifest",
            ));
        }
        verify_metadata(active, &published)
    }
}

pub(super) fn rollback(
    active: &mut File,
    manifest: &Path,
    snapshot: &File,
    displaced: &Path,
) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::io::{Read as _, Seek as _, Write as _};

        let _ = displaced;
        let mut original = Vec::new();
        let mut snapshot = snapshot;
        snapshot.rewind()?;
        snapshot.read_to_end(&mut original)?;
        active.rewind()?;
        active.write_all(&original)?;
        active.set_len(original.len() as u64)?;
        active.sync_all()?;
        active.rewind()?;
        let mut verified = Vec::new();
        active.read_to_end(&mut verified)?;
        if verified != original || !path_names_file(active, manifest)? {
            return Err(std::io::Error::other(
                "rollback bytes or identity did not verify",
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = snapshot;
        match exchange(active, manifest, displaced, displaced) {
            Ok(()) => Ok(()),
            Err(ExchangeError::Io(error)) => Err(error),
            #[cfg(windows)]
            Err(ExchangeError::AfterMutation(error)) => Err(error),
            Err(ExchangeError::Unsupported(reason)) => {
                Err(std::io::Error::new(std::io::ErrorKind::Unsupported, reason))
            }
        }
    }
}

pub(super) fn path_names_file(file: &File, path: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let opened = file.metadata()?;
        let current = std::fs::metadata(path)?;
        Ok(opened.dev() == current.dev() && opened.ino() == current.ino())
    }
    #[cfg(windows)]
    {
        let opened = windows_file_identity(file)?;
        let current = open_published(path)?;
        Ok(opened == windows_file_identity(&current)?)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, path);
        Ok(false)
    }
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> std::io::Result<(u32, u64)> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a valid handle and the API initializes the output on success.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr()) }
        == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the successful API call initialized the structure.
    let information = unsafe { information.assume_init() };
    Ok((
        information.dwVolumeSerialNumber,
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ))
}

pub(super) fn copy_metadata(source: &File, destination: &File) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd as _;
        // SAFETY: both descriptors remain valid for the duration of the metadata-only copy.
        if unsafe {
            libc::fcopyfile(
                source.as_raw_fd(),
                destination.as_raw_fd(),
                std::ptr::null_mut(),
                libc::COPYFILE_METADATA,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        use xattr::FileExt as _;
        let metadata = source.metadata()?;
        // SAFETY: destination owns a live fd; uid/gid originate from source metadata.
        if unsafe { libc::fchown(destination.as_raw_fd(), metadata.uid(), metadata.gid()) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EPERM) {
                return Err(error);
            }
            let actual = destination.metadata()?;
            if actual.uid() != metadata.uid() || actual.gid() != metadata.gid() {
                return Err(error);
            }
        }
        destination.set_permissions(std::fs::Permissions::from_mode(metadata.mode()))?;
        for name in source.list_xattr()? {
            let value = source.get_xattr(&name)?.ok_or_else(|| {
                std::io::Error::other("extended attribute disappeared during metadata copy")
            })?;
            destination.set_xattr(&name, &value)?;
        }
    }
    #[cfg(windows)]
    {
        let _ = (source, destination);
    }
    Ok(())
}

pub(super) fn protect_recovery(source: &File, recovery: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = source;
        recovery.set_permissions(std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle as _;
            use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
            use windows_sys::Win32::Security::Authorization::{
                GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo,
            };
            use windows_sys::Win32::Security::{
                ACL, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION,
                OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            };

            let mut owner: PSID = std::ptr::null_mut();
            let mut group: PSID = std::ptr::null_mut();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let information =
                OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
            // SAFETY: all output pointers are valid and `source` owns a live file handle.
            let read = unsafe {
                GetSecurityInfo(
                    source.as_raw_handle() as _,
                    SE_FILE_OBJECT,
                    information,
                    &mut owner,
                    &mut group,
                    &mut dacl,
                    std::ptr::null_mut(),
                    &mut descriptor,
                )
            };
            if read != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(read as i32));
            }
            // SAFETY: owner, group, and dacl point into `descriptor`, which remains allocated for
            // this call; `recovery` owns a live destination handle.
            let written = unsafe {
                SetSecurityInfo(
                    recovery.as_raw_handle() as _,
                    SE_FILE_OBJECT,
                    information,
                    owner,
                    group,
                    dacl,
                    std::ptr::null(),
                )
            };
            // SAFETY: `descriptor` was allocated by `GetSecurityInfo` on success.
            unsafe { LocalFree(descriptor.cast()) };
            if written != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(written as i32));
            }
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (source, recovery);
            Ok(())
        }
    }
}

pub(super) fn verify_metadata(source: &File, destination: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let source_metadata = source.metadata()?;
        let destination_metadata = destination.metadata()?;
        if source_metadata.mode() != destination_metadata.mode()
            || source_metadata.uid() != destination_metadata.uid()
            || source_metadata.gid() != destination_metadata.gid()
        {
            return Err(std::io::Error::other(
                "manifest ownership or mode was not preserved",
            ));
        }
        let source_xattrs = xattrs(source)?;
        let destination_xattrs = xattrs(destination)?;
        if source_xattrs != destination_xattrs {
            return Err(std::io::Error::other(
                "manifest extended attributes were not preserved",
            ));
        }
    }
    #[cfg(windows)]
    {
        let _ = (source, destination);
    }
    Ok(())
}

#[cfg(unix)]
fn xattrs(file: &File) -> std::io::Result<std::collections::BTreeMap<std::ffi::OsString, Vec<u8>>> {
    use xattr::FileExt as _;
    let mut attributes = std::collections::BTreeMap::new();
    for name in file.list_xattr()? {
        let value = file.get_xattr(&name)?.ok_or_else(|| {
            std::io::Error::other("extended attribute disappeared during verification")
        })?;
        attributes.insert(name, value);
    }
    Ok(attributes)
}
