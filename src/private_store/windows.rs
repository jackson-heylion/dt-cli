//! Windows metadata is owned by the current SID with a protected, user-only DACL.
//! ACL operations use non-following file handles; directories start private at creation.
use super::unavailable;
use crate::output::Result;
use std::{
    ffi::c_void,
    fs::{self, File, OpenOptions},
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        self, ACL, Authorization::*, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        SECURITY_DESCRIPTOR, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::{
        CreateDirectoryW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, READ_CONTROL, WRITE_DAC,
        WRITE_OWNER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}
/// Word-aligned buffer keeps the SID returned inside TOKEN_USER alive for all API calls.
struct User(Vec<usize>);
impl User {
    fn current() -> Result<Self> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(unavailable());
            }
            let token = Token(token);
            let mut required = 0;
            Security::GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
            if required < size_of::<TOKEN_USER>() as u32 || required > 65536 {
                return Err(unavailable());
            }
            let mut storage = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
            if Security::GetTokenInformation(
                token.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                required,
                &mut required,
            ) == 0
            {
                return Err(unavailable());
            }
            let user = Self(storage);
            if Security::IsValidSid(user.sid()) == 0 {
                return Err(unavailable());
            }
            Ok(user)
        }
    }
    fn sid(&self) -> PSID {
        unsafe { (*(self.0.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}
fn open(path: &Path, access: u32) -> Result<File> {
    let file = OpenOptions::new()
        .access_mode(access)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|_| unavailable())?;
    if file
        .metadata()
        .map_err(|_| unavailable())?
        .file_attributes()
        & FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(unavailable());
    }
    Ok(file)
}
fn acl(user: &User, directory: bool) -> Result<Local> {
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: if directory {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            0
        },
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: user.sid().cast(),
            ..Default::default()
        },
    };
    unsafe {
        let mut acl = null_mut();
        if SetEntriesInAclW(1, &entry, null(), &mut acl) != 0 || acl.is_null() {
            return Err(unavailable());
        }
        Ok(Local(acl.cast()))
    }
}
pub(super) fn validate(path: &Path) -> Result<()> {
    let file = open(path, READ_CONTROL)?;
    let user = User::current()?;
    unsafe {
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
            return Err(unavailable());
        }
        let descriptor = Local(descriptor);
        let (mut control, mut revision) = (0, 0);
        if owner.is_null()
            || Security::IsValidSid(owner) == 0
            || Security::EqualSid(owner, user.sid()) == 0
            || dacl.is_null()
            || Security::IsValidAcl(dacl) == 0
            || Security::GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision)
                == 0
            || control & SE_DACL_PROTECTED == 0
        {
            return Err(unavailable());
        }
        let (mut count, mut entries) = (0, null_mut());
        if GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) != 0 {
            return Err(unavailable());
        }
        let _entries = Local(entries.cast());
        if count == 0 || count > 64 || entries.is_null() {
            return Err(unavailable());
        }
        for entry in std::slice::from_raw_parts(entries, count as usize) {
            if !matches!(entry.grfAccessMode, GRANT_ACCESS | SET_ACCESS)
                || entry.Trustee.TrusteeForm != TRUSTEE_IS_SID
                || entry.Trustee.ptstrName.is_null()
                || Security::IsValidSid(entry.Trustee.ptstrName.cast()) == 0
                || Security::EqualSid(entry.Trustee.ptstrName.cast(), user.sid()) == 0
            {
                return Err(unavailable());
            }
        }
    }
    Ok(())
}
pub(super) fn protect_new(path: &Path) -> Result<()> {
    let file = open(path, READ_CONTROL | WRITE_DAC | WRITE_OWNER)?;
    let user = User::current()?;
    let acl = acl(&user, file.metadata().map_err(|_| unavailable())?.is_dir())?;
    unsafe {
        if SetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            user.sid(),
            null_mut(),
            acl.0.cast::<ACL>(),
            null(),
        ) != 0
        {
            return Err(unavailable());
        }
    }
    validate(path)
}
pub(super) fn create_directory(path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(unavailable)?;
    fs::create_dir_all(parent).map_err(|_| unavailable())?;
    let user = User::current()?;
    let acl = acl(&user, true)?;
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast::<c_void>();
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err(unavailable());
    }
    name.push(0);
    unsafe {
        if Security::InitializeSecurityDescriptor(descriptor_ptr, 1) == 0
            || Security::SetSecurityDescriptorOwner(descriptor_ptr, user.sid(), 0) == 0
            || Security::SetSecurityDescriptorDacl(descriptor_ptr, 1, acl.0.cast(), 0) == 0
            || Security::SetSecurityDescriptorControl(
                descriptor_ptr,
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            ) == 0
        {
            return Err(unavailable());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor_ptr,
            bInheritHandle: 0,
        };
        if CreateDirectoryW(name.as_ptr(), &attributes) == 0 {
            // A concurrent creator is accepted only if it produced the same private policy.
            return validate(path);
        }
    }
    validate(path)
}
#[cfg(test)]
mod tests;
