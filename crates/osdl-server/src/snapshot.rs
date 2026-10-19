use osdl_core::media::MAX_SNAPSHOT_BYTES;
use std::path::{Path, PathBuf};
use tonic::Status;

pub(super) fn snapshot_bytes(root: &Path, requested: &str) -> Result<Vec<u8>, Status> {
    reject_links(root)?;
    let root = std::fs::canonicalize(root)
        .map_err(|_| Status::failed_precondition("snapshot unavailable"))?;
    let candidate = PathBuf::from(requested);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    reject_links(&candidate)?;
    let candidate = std::fs::canonicalize(candidate)
        .map_err(|_| Status::failed_precondition("snapshot unavailable"))?;
    if !candidate.starts_with(&root) {
        return Err(Status::failed_precondition("snapshot path rejected"));
    }
    let metadata = std::fs::symlink_metadata(&candidate)
        .map_err(|_| Status::failed_precondition("snapshot unavailable"))?;
    if !metadata.is_file() || metadata.len() > MAX_SNAPSHOT_BYTES as u64 {
        return Err(Status::failed_precondition("snapshot image rejected"));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(candidate)
        .map_err(|_| Status::failed_precondition("snapshot unavailable"))?;
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_SNAPSHOT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Status::failed_precondition("snapshot unavailable"))?;
    if bytes.len() > MAX_SNAPSHOT_BYTES || !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Err(Status::failed_precondition("snapshot image rejected"));
    }
    Ok(bytes)
}

fn reject_links(path: &Path) -> Result<(), Status> {
    let mut checked = PathBuf::new();
    for part in path.components() {
        checked.push(part);
        if matches!(
            part,
            std::path::Component::Prefix(_) | std::path::Component::RootDir
        ) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&checked)
            .map_err(|_| Status::failed_precondition("snapshot unavailable"))?;
        if metadata.file_type().is_symlink() {
            return Err(Status::failed_precondition("snapshot path rejected"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(Status::failed_precondition("snapshot path rejected"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_confined_bounded_regular_images() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("snapshots");
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        std::fs::write(root.join("one.jpg"), [0xff, 0xd8, 0xff, 1]).unwrap();
        std::fs::write(temp.path().join("outside.jpg"), [0xff, 0xd8, 0xff, 2]).unwrap();
        assert_eq!(
            snapshot_bytes(&root, "one.jpg").unwrap(),
            [0xff, 0xd8, 0xff, 1]
        );
        assert!(snapshot_bytes(&root, "../outside.jpg").is_err());
        assert!(snapshot_bytes(&root, ".").is_err());
        std::fs::write(root.join("text.jpg"), b"secret text").unwrap();
        assert!(snapshot_bytes(&root, "text.jpg").is_err());
        let mut large = vec![0; 600 * 1024];
        large[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
        std::fs::write(root.join("large.jpg"), &large).unwrap();
        assert_eq!(snapshot_bytes(&root, "large.jpg").unwrap(), large);
        let oversized = std::fs::File::create(root.join("oversized.jpg")).unwrap();
        oversized.set_len(MAX_SNAPSHOT_BYTES as u64 + 1).unwrap();
        assert!(snapshot_bytes(&root, "oversized.jpg").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn snapshots_reject_symlinked_roots_and_members() {
        let temp = tempfile::tempdir().unwrap();
        let actual = temp.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        std::fs::write(actual.join("one.jpg"), [0xff, 0xd8, 0xff]).unwrap();
        std::os::unix::fs::symlink(&actual, temp.path().join("snapshots")).unwrap();
        assert!(snapshot_bytes(&temp.path().join("snapshots"), "one.jpg").is_err());
        std::os::unix::fs::symlink(actual.join("one.jpg"), actual.join("alias.jpg")).unwrap();
        assert!(snapshot_bytes(&actual, "alias.jpg").is_err());
    }
}
