use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl,
    GetTokenInformation, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SetFileSecurityW, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::core::PWSTR;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const LOCAL_SYSTEM_SID: &str = "S-1-5-18";

pub(super) fn ensure_private_directory(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    let mut target_created = false;

    for component in path.components() {
        match component {
            // A drive/UNC prefix and its root are syntax, not directories to query as
            // incomplete paths (notably `C:` means a per-drive working directory).
            Component::Prefix(_) | Component::RootDir => {
                current.push(component.as_os_str());
                continue;
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private application directory must not contain '..'",
                ));
            }
            Component::Normal(_) => current.push(component.as_os_str()),
        }

        let target = current.components().eq(path.components());
        let (metadata, created) = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => (metadata, false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match std::fs::create_dir(&current) {
                    Ok(()) => (std::fs::symlink_metadata(&current)?, true),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        (std::fs::symlink_metadata(&current)?, false)
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };

        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(permission_denied(format!(
                "refusing reparse point in application directory: {}",
                current.display()
            )));
        }

        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!(
                    "application directory path is not a directory: {}",
                    current.display()
                ),
            ));
        }

        if target {
            target_created = created;
        }
    }

    if target_created {
        apply_private_acl(path)?;
    }

    verify_private_acl(path)
}

fn permission_denied(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}

fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn apply_private_acl(path: &Path) -> io::Result<()> {
    let user_sid = current_user_sid_string()?;
    let sddl = format!("D:P(A;OICI;FA;;;{user_sid})(A;OICI;FA;;;SY)");

    apply_dacl(path, &sddl)
}

fn apply_dacl(path: &Path, sddl: &str) -> io::Result<()> {
    let encoded_sddl: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let encoded_path = wide_null(path.as_os_str());
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();

    // SAFETY: the UTF-16 input is NUL-terminated; Windows allocates the returned
    // self-relative descriptor, which is released with LocalFree below.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            encoded_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: both arguments remain valid for the duration of this call.
    let result = unsafe {
        SetFileSecurityW(
            encoded_path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    // SAFETY: descriptor is the LocalAlloc allocation returned above.
    unsafe { LocalFree(descriptor) };

    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

fn verify_private_acl(path: &Path) -> io::Result<()> {
    let user_sid = current_user_sid_string()?;
    let encoded_path = wide_null(path.as_os_str());
    let mut owner: PSID = ptr::null_mut();
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();

    // SAFETY: the path is NUL-terminated and all output pointers are valid. owner and
    // dacl point inside descriptor, which remains allocated throughout validation.
    let status = unsafe {
        GetNamedSecurityInfoW(
            encoded_path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };

    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }

    let checked = validate_descriptor(path, descriptor, owner, dacl, &user_sid);

    // SAFETY: descriptor is the LocalAlloc allocation returned above.
    unsafe { LocalFree(descriptor) };

    checked
}

fn validate_descriptor(
    path: &Path,
    descriptor: PSECURITY_DESCRIPTOR,
    owner: PSID,
    dacl: *mut ACL,
    user_sid: &str,
) -> io::Result<()> {
    if owner.is_null() || sid_to_string(owner)? != user_sid {
        return Err(permission_denied(format!(
            "application directory is not owned by the current user: {}",
            path.display()
        )));
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: descriptor is a valid security descriptor returned by Windows.
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 || dacl.is_null() {
        return Err(permission_denied(format!(
            "application directory does not have a protected private ACL: {}",
            path.display()
        )));
    }

    let ace_count = unsafe { (*dacl).AceCount };
    let mut grants_owner = false;

    for index in 0..u32::from(ace_count) {
        let mut raw_ace: *mut c_void = ptr::null_mut();
        // SAFETY: dacl is valid and index is bounded by its AceCount.
        if unsafe { GetAce(dacl, index, &mut raw_ace) } == 0 {
            return Err(io::Error::last_os_error());
        }

        // Only simple allow ACEs for this user and LocalSystem are accepted. Deny,
        // callback, object, and unrelated-principal ACEs fail closed.
        let ace = raw_ace.cast::<ACCESS_ALLOWED_ACE>();
        if unsafe { (*ace).Header.AceType } != ACCESS_ALLOWED_ACE_TYPE {
            return Err(permission_denied(format!(
                "application directory ACL contains an unsupported entry: {}",
                path.display()
            )));
        }

        let sid = unsafe { ptr::addr_of_mut!((*ace).SidStart).cast::<c_void>() };
        let sid = sid_to_string(sid)?;
        if sid == user_sid {
            grants_owner |= unsafe { (*ace).Mask } & FILE_ALL_ACCESS == FILE_ALL_ACCESS;
        } else if sid != LOCAL_SYSTEM_SID {
            return Err(permission_denied(format!(
                "application directory ACL grants another principal access: {}",
                path.display()
            )));
        }
    }

    if !grants_owner {
        return Err(permission_denied(format!(
            "application directory ACL does not grant its owner access: {}",
            path.display()
        )));
    }

    Ok(())
}

fn current_user_sid_string() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: token points to writable storage; the pseudo process handle is valid.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let result = token_user_sid_string(token);

    // SAFETY: token is an owned handle returned by OpenProcessToken.
    unsafe { CloseHandle(token) };

    result
}

fn token_user_sid_string(token: HANDLE) -> io::Result<String> {
    let mut required = 0u32;
    // The first call obtains the required size and is expected to fail with
    // ERROR_INSUFFICIENT_BUFFER.
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0u8; required as usize];
    // SAFETY: buffer has the exact byte capacity requested by Windows.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    let user = buffer.as_ptr().cast::<TOKEN_USER>();
    // SAFETY: a successful TokenUser query initialized TOKEN_USER and its SID.
    sid_to_string(unsafe { (*user).User.Sid })
}

fn sid_to_string(sid: PSID) -> io::Result<String> {
    let mut value: PWSTR = ptr::null_mut();
    // SAFETY: sid is supplied by a successful Windows security API.
    if unsafe { ConvertSidToStringSidW(sid, &mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let len = unsafe {
        let mut len = 0;
        while *value.add(len) != 0 {
            len += 1;
        }
        len
    };
    // SAFETY: value points to a NUL-terminated allocation of `len` UTF-16 units.
    let string = String::from_utf16(unsafe { std::slice::from_raw_parts(value, len) })
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Windows returned invalid SID"));
    // SAFETY: value is the LocalAlloc allocation returned above.
    unsafe { LocalFree(value.cast()) };

    string
}

#[cfg(test)]
pub(super) fn apply_world_access_for_test(path: &Path) -> io::Result<()> {
    apply_dacl(path, "D:P(A;OICI;FA;;;WD)")
}
