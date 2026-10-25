use anyhow::{ensure, Result};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[cfg(unix)]
#[path = "store/unix.rs"]
mod native;
#[cfg(windows)]
#[path = "store/windows.rs"]
mod native;
/// Explicit caller-selected namespace; standalone and Runner roots must be distinct.
pub struct ProfileStore {
    root: PathBuf,
}
impl ProfileStore {
    pub fn open(root: PathBuf) -> Result<Self> {
        ensure!(root.is_absolute(), "private_store_requires_absolute_path");
        for path in root.ancestors().filter(|p| p.exists()) {
            ensure!(
                !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
                "unsafe_private_path"
            );
            native::no_reparse(path)?;
        }
        if !root.exists() {
            native::create_dir(&root)?;
        }
        native::verify(&root, true)?;
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        ensure!(
            !name.is_empty()
                && name.len() <= 128
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "invalid_profile_name"
        );
        Ok(self.root.join(name))
    }
    pub fn read_secret(&self, name: &str) -> Result<Vec<u8>> {
        native::verify(&self.root, true)?;
        let path = self.path(name)?;
        native::verify(&path, false)?;
        let mut file = native::open_read(&path)?;
        ensure!(file.metadata()?.len() <= 65536, "private_record_too_large");
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        native::decrypt(&bytes)
    }
    pub fn write_secret(&self, name: &str, bytes: &[u8], replace: bool) -> Result<()> {
        ensure!(bytes.len() <= 32768, "private_record_too_large");
        native::verify(&self.root, true)?;
        let path = self.path(name)?;
        if path.exists() {
            native::verify(&path, false)?;
            ensure!(replace, "profile_exists");
        }
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        native::protect(temp.path())?;
        temp.write_all(&native::encrypt(bytes)?)?;
        native::publish(temp, &path, replace)?;
        native::verify(&path, false)?;
        Ok(())
    }
    /// Cross-process single writer ownership for a selected laboratory or profile.
    pub fn lock(&self, name: &str) -> Result<std::fs::File> {
        let path = self.path(name)?;
        if !path.exists() {
            match self.write_secret(name, b"", false) {
                Ok(()) => {}
                Err(e) if !path.exists() => return Err(e),
                Err(_) => {}
            }
        }
        native::verify(&path, false)?;
        let file = native::open_lock(&path)?;
        file.try_lock()
            .map_err(|_| anyhow::anyhow!("private_record_in_use"))?;
        Ok(file)
    }
    pub fn contains(&self, name: &str) -> Result<bool> {
        Ok(self.path(name)?.try_exists()?)
    }
}
