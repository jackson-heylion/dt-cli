//! Preserve WorkBuddy's existing Windows ACL when replacing its settings file.
use super::{Result, failure};
use std::{
    fs::{File, OpenOptions},
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{
        self,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER, TokenUser,
        UNPROTECTED_DACL_SECURITY_INFORMATION, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileInformationByHandle, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    },
    System::{
        SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

#[derive(PartialEq)]
pub(super) struct Snapshot {
    volume: u32,
    high: u32,
    low: u32,
    attributes: u32,
    pub(super) inheritance_protected: bool,
}

fn open(path: &Path, directory: bool) -> Result<(File, Snapshot)> {
    let file = OpenOptions::new()
        .access_mode(if directory {
            READ_CONTROL
        } else {
            FILE_GENERIC_READ
        })
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|_| failure("settings"))?;
    let meta = file.metadata().map_err(|_| failure("settings"))?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(failure("settings"));
    }
    let inheritance_protected = validate_acl(&file)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) } == 0 {
        return Err(failure("settings"));
    }
    Ok((
        file,
        Snapshot {
            volume: info.dwVolumeSerialNumber,
            high: info.nFileIndexHigh,
            low: info.nFileIndexLow,
            attributes: info.dwFileAttributes,
            inheritance_protected,
        },
    ))
}

pub(super) fn validate_parent(path: &Path) -> Result<()> {
    open(path, true).map(|_| ())
}
pub(super) fn open_settings(path: &Path) -> Result<(File, Snapshot)> {
    open(path, false)
}

fn validate_acl(file: &File) -> Result<bool> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(failure("owner"));
        }
        let mut required = 0;
        Security::GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required);
        if required < std::mem::size_of::<TOKEN_USER>() as u32 || required > 65536 {
            CloseHandle(token);
            return Err(failure("owner"));
        }
        let mut user = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = Security::GetTokenInformation(
            token,
            TokenUser,
            user.as_mut_ptr().cast(),
            required,
            &mut required,
        ) != 0;
        CloseHandle(token);
        if !ok {
            return Err(failure("owner"));
        }
        let current_sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let (mut owner, mut dacl, mut descriptor) = (null_mut(), null_mut(), null_mut());
        if GetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        ) != 0
        {
            return Err(failure("permissions"));
        }
        let result = (|| {
            if owner.is_null()
                || Security::IsValidSid(owner) == 0
                || (Security::EqualSid(owner, current_sid) == 0
                    && Security::IsWellKnownSid(owner, WinBuiltinAdministratorsSid) == 0
                    && Security::IsWellKnownSid(owner, WinLocalSystemSid) == 0)
                || dacl.is_null()
                || Security::IsValidAcl(dacl) == 0
            {
                return Err(failure("permissions"));
            }
            // Standard inherited ACLs are accepted; only this user, SYSTEM and admins may mutate.
            for index in 0..(*dacl).AceCount as u32 {
                let mut raw = null_mut();
                if Security::GetAce(dacl, index, &mut raw) == 0 || raw.is_null() {
                    return Err(failure("permissions"));
                }
                let header = &*raw.cast::<Security::ACE_HEADER>();
                if header.AceFlags & Security::INHERIT_ONLY_ACE as u8 != 0 {
                    continue;
                }
                if header.AceType == ACCESS_DENIED_ACE_TYPE as u8 {
                    continue;
                }
                if header.AceType != ACCESS_ALLOWED_ACE_TYPE as u8 {
                    return Err(failure("permissions"));
                }
                let ace = &*raw.cast::<Security::ACCESS_ALLOWED_ACE>();
                let sid = (&ace.SidStart as *const u32).cast_mut().cast();
                if Security::IsValidSid(sid) == 0 {
                    return Err(failure("permissions"));
                }
                const MUTATION: u32 = 0x10000000 | 0x40000000 | 0x000d0156;
                if ace.Mask & MUTATION != 0
                    && Security::EqualSid(sid, current_sid) == 0
                    && Security::IsWellKnownSid(sid, WinLocalSystemSid) == 0
                    && Security::IsWellKnownSid(sid, WinBuiltinAdministratorsSid) == 0
                {
                    return Err(failure("permissions"));
                }
            }
            let (mut control, mut revision) = (0, 0);
            if Security::GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0
            {
                return Err(failure("permissions"));
            }
            Ok(control & SE_DACL_PROTECTED != 0)
        })();
        LocalFree(descriptor);
        result
    }
}

pub(super) fn replace_settings(
    temporary: tempfile::NamedTempFile,
    path: &Path,
    original: File,
) -> Result<()> {
    let replacement = OpenOptions::new()
        .access_mode(WRITE_DAC | WRITE_OWNER)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(temporary.path())
        .map_err(|_| failure("permissions"))?;
    unsafe {
        let mut descriptor = null_mut();
        if GetSecurityInfo(
            original.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        ) != 0
        {
            return Err(failure("permissions"));
        }
        let (mut control, mut revision) = (0, 0);
        let valid =
            Security::GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) != 0;
        let protection = if control & SE_DACL_PROTECTED != 0 {
            PROTECTED_DACL_SECURITY_INFORMATION
        } else {
            UNPROTECTED_DACL_SECURITY_INFORMATION
        };
        let copied = valid
            && Security::SetKernelObjectSecurity(
                replacement.as_raw_handle().cast(),
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | protection,
                descriptor,
            ) != 0;
        LocalFree(descriptor);
        if !copied {
            return Err(failure("permissions"));
        }
    }
    drop(replacement);
    // Windows can refuse replacement while the destination retains a byte-range lock.
    // Content and identity were checked under that lock immediately before this call.
    original.unlock().map_err(|_| failure("unlock"))?;
    drop(original);
    let temporary = temporary.into_temp_path();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    // Move the prepared ACL unchanged; ReplaceFile would merge inherited permissions again.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        let code = std::io::Error::last_os_error().raw_os_error();
        let mut error = failure("atomic_replace");
        error.details.as_mut().unwrap()["osCode"] = serde_json::json!(code);
        return Err(error);
    }
    Ok(())
}
