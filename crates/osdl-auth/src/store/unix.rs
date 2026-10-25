use anyhow::{ensure, Result};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
pub fn no_reparse(_: &Path) -> Result<()> {
    Ok(())
}
pub fn create_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
pub fn verify(path: &Path, dir: bool) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        !m.file_type().is_symlink() && m.is_dir() == dir && (dir || m.is_file()),
        "unsafe_private_path"
    );
    ensure!(
        m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o777 == if dir { 0o700 } else { 0o600 },
        "unsafe_private_permissions"
    );
    if !dir {
        ensure!(m.nlink() == 1, "unsafe_private_links");
    }
    Ok(())
}
pub fn open_read(path: &Path) -> Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o777 == 0o600 && m.nlink() == 1,
        "unsafe_private_permissions"
    );
    Ok(f)
}
pub fn protect(path: &Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
pub fn encrypt(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
pub fn decrypt(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
pub fn open_lock(path: &Path) -> Result<std::fs::File> {
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}

pub fn publish(temp: tempfile::NamedTempFile, path: &Path, replace: bool) -> Result<()> {
    ensure!(
        temp.path().parent() == path.parent(),
        "private_publication_requires_same_directory"
    );
    temp.as_file().sync_all()?;
    if replace {
        temp.persist(path)?;
    } else {
        temp.persist_noclobber(path)?;
    }
    File::open(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("invalid_private_path"))?,
    )?
    .sync_all()?;
    Ok(())
}
