use anyhow::{ensure, Result};
use std::{
    ffi::c_void,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{Authorization::*, Cryptography::*, *},
    Storage::FileSystem::*,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn check(ok: i32) -> Result<()> {
    ensure!(
        ok != 0,
        "private_store_native_failure: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
fn user() -> Result<Vec<usize>> {
    unsafe {
        let mut token = null_mut();
        check(OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY,
            &mut token,
        ))?;
        let mut size = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
        let mut bytes = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        let result =
            GetTokenInformation(token, TokenUser, bytes.as_mut_ptr().cast(), size, &mut size);
        CloseHandle(token);
        check(result)?;
        Ok(bytes)
    }
}
fn sid(bytes: &[usize]) -> PSID {
    unsafe { (*(bytes.as_ptr().cast::<TOKEN_USER>())).User.Sid }
}
fn descriptor() -> Result<Local> {
    unsafe {
        let user = user()?;
        let mut text = null_mut();
        check(ConvertSidToStringSidW(sid(&user), &mut text))?;
        let owner = Local(text.cast());
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let name = String::from_utf16(std::slice::from_raw_parts(text, len))?;
        drop(owner);
        let sddl: Vec<u16> = format!("O:{name}D:P(A;OICI;FA;;;{name})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut desc = null_mut();
        check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut desc,
            null_mut(),
        ))?;
        Ok(Local(desc))
    }
}
pub fn no_reparse(path: &Path) -> Result<()> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "unsafe_private_path"
    );
    Ok(())
}
pub fn create_dir(path: &Path) -> Result<()> {
    unsafe {
        let desc = descriptor()?;
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: desc.0,
            bInheritHandle: 0,
        };
        check(CreateDirectoryW(wide(path).as_ptr(), &attrs))
    }
}
pub fn protect(path: &Path) -> Result<()> {
    unsafe {
        let desc = descriptor()?;
        let mut acl = null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        check(GetSecurityDescriptorDacl(
            desc.0,
            &mut present,
            &mut acl,
            &mut defaulted,
        ))?;
        let mut owner = null_mut();
        check(GetSecurityDescriptorOwner(
            desc.0,
            &mut owner,
            &mut defaulted,
        ))?;
        let code = SetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            null_mut(),
            acl,
            null(),
        );
        ensure!(code == 0, "private_store_dacl_failure");
        Ok(())
    }
}
pub fn verify(path: &Path, dir: bool) -> Result<()> {
    no_reparse(path)?;
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() == dir && (dir || m.is_file()),
        "unsafe_private_path"
    );
    unsafe {
        let user = user()?;
        let mut owner = null_mut();
        let mut acl = null_mut();
        let mut desc = null_mut();
        let code = GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut desc,
        );
        ensure!(code == 0, "private_store_dacl_failure");
        let _memory = Local(desc);
        ensure!(
            !owner.is_null() && EqualSid(owner, sid(&user)) != 0 && !acl.is_null(),
            "unsafe_private_owner"
        );
        let mut control = 0;
        let mut revision = 0;
        check(GetSecurityDescriptorControl(
            desc,
            &mut control,
            &mut revision,
        ))?;
        ensure!(
            control & SE_DACL_PROTECTED != 0 && (*acl).AceCount == 1,
            "unsafe_private_dacl"
        );
        let mut ace = null_mut();
        check(GetAce(acl, 0, &mut ace))?;
        let ace = &*(ace.cast::<ACCESS_ALLOWED_ACE>());
        ensure!(
            ace.Header.AceType == 0
                && EqualSid((&ace.SidStart as *const u32).cast_mut().cast(), sid(&user)) != 0
                && ace.Mask & FILE_ALL_ACCESS == FILE_ALL_ACCESS,
            "unsafe_private_dacl"
        );
    }
    Ok(())
}
pub fn open_read(path: &Path) -> Result<std::fs::File> {
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?)
}
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };
        let ok = if encrypt {
            CryptProtectData(
                &input,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        check(ok)?;
        let _memory = Local(output.pbData.cast());
        Ok(std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec())
    }
}
pub fn encrypt(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, true)
}
pub fn decrypt(bytes: &[u8]) -> Result<Vec<u8>> {
    crypt(bytes, false)
}
pub fn open_lock(path: &Path) -> Result<std::fs::File> {
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?)
}

pub fn publish(mut temp: tempfile::NamedTempFile, path: &Path, replace: bool) -> Result<()> {
    ensure!(
        temp.path().parent() == path.parent(),
        "private_publication_requires_same_directory"
    );
    // tempfile 3.27 persist omits WRITE_THROUGH. Clear its temporary attribute before
    // flushing bytes, then publish the same-volume name durably without a copy fallback.
    let source = wide(temp.path());
    unsafe {
        check(SetFileAttributesW(source.as_ptr(), FILE_ATTRIBUTE_NORMAL))?;
    }
    temp.as_file().sync_all()?;
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    unsafe {
        check(MoveFileExW(source.as_ptr(), wide(path).as_ptr(), flags))?;
    }
    temp.disable_cleanup(true);
    Ok(())
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use std::io::Write;
    fn candidate(root: &Path, bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut temp = tempfile::NamedTempFile::new_in(root).unwrap();
        protect(temp.path()).unwrap();
        temp.write_all(bytes).unwrap();
        temp
    }
    #[test]
    fn native_publication_creates_replaces_and_preserves_destination_on_failure() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let store = crate::ProfileStore::open(root.path().join("private")).unwrap();
        let destination = store.root().join("record");
        publish(candidate(store.root(), b"initial"), &destination, false).unwrap();
        verify(&destination, false).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"initial");
        assert_eq!(
            std::fs::metadata(&destination).unwrap().file_attributes() & FILE_ATTRIBUTE_TEMPORARY,
            0
        );
        assert!(publish(candidate(store.root(), b"no clobber"), &destination, false).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"initial");
        publish(candidate(store.root(), b"replacement"), &destination, true).unwrap();
        verify(&destination, false).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"replacement");
        let locked = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&destination)
            .unwrap();
        assert!(publish(candidate(store.root(), b"blocked"), &destination, true).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"replacement");
        assert_eq!(
            std::fs::read_dir(store.root()).unwrap().count(),
            1,
            "failed publication must clean its private temporary file"
        );
        drop(locked);
    }
}
