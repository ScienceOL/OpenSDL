use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

pub struct AssetFixture {
    _temp: TempDir,
    pub root: PathBuf,
}

impl AssetFixture {
    pub fn visual() -> Self {
        let temp = tempfile::tempdir().expect("create asset fixture directory");
        let root = temp.path().join("ur5e");
        fs::create_dir(&root).expect("create asset root");
        fs::copy(fixture_dir().join("asset.yaml"), root.join("asset.yaml")).expect("copy manifest");
        fs::copy(fixture_dir().join("LICENSE"), root.join("LICENSE")).expect("copy license");
        fs::write(root.join("scene.usda"), b"#usda 1.0\n").expect("write OpenUSD source");
        write_minimal_glb(&root.join("preview.glb"), b"{}\0\0");
        fs::write(root.join("thumbnail.png"), minimal_png()).expect("write thumbnail");
        Self { _temp: temp, root }
    }

    #[allow(dead_code)]
    pub fn replace_manifest(&self, manifest: &str) {
        fs::write(self.root.join("asset.yaml"), manifest).expect("replace manifest");
    }
}

#[allow(dead_code)]
pub fn base_manifest() -> String {
    fs::read_to_string(fixture_dir().join("asset.yaml")).expect("read fixture manifest")
}

pub fn write_minimal_glb(path: &Path, json_chunk: &[u8]) {
    assert_eq!(json_chunk.len() % 4, 0, "GLB chunks are four-byte aligned");
    let total_length = 12 + 8 + json_chunk.len();
    let mut bytes = Vec::with_capacity(total_length);
    bytes.extend_from_slice(b"glTF");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&(total_length as u32).to_le_bytes());
    bytes.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0x4E4F534A_u32.to_le_bytes());
    bytes.extend_from_slice(json_chunk);
    fs::write(path, bytes).expect("write minimal GLB");
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/visual")
}

fn minimal_png() -> &'static [u8] {
    &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D',
        b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
    ]
}
