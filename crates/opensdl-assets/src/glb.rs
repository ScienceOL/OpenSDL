//! Structural validation for the self-contained glTF 2.0 binary payload.
//!
//! The OpenSDL declarations in `extras` describe presentation state; they do
//! not make a malformed glTF asset valid. This module therefore validates the
//! container, JSON object graph, embedded resources, and accessor bounds before
//! the presentation declarations are inspected. Portable previews deliberately
//! use only the standard GLB BIN chunk: external files and data URIs would make
//! the independently distributed `preview.glb` incomplete.

use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::AssetError;

const GLB_MAGIC: u32 = 0x4654_6c67;
const JSON_CHUNK: u32 = 0x4e4f_534a;
const BIN_CHUNK: u32 = 0x004e_4942;
/// A portable delivery preview is bounded before any allocation. Authoring USD
/// remains the place for high-fidelity geometry; browser-distributed GLBs must
/// not require unbounded memory merely to inspect their JSON declarations.
const MAX_GLB_BYTES: usize = 256 * 1024 * 1024;

/// Reads and validates a self-contained glTF 2.0 binary file.
///
/// In addition to the container and JSON graph, this checks scene/node indices,
/// mesh accessor references, buffer-view ranges, and accessor strides. It does
/// not claim to be a replacement for the complete Khronos glTF validator: GPU
/// material semantics and renderer-specific extensions remain renderer checks.
pub(crate) fn validate(path: &Path) -> Result<()> {
    let file = std::fs::File::open(path).map_err(|source| AssetError::read(path, source))?;
    if file
        .metadata()
        .map_err(|source| AssetError::read(path, source))?
        .len()
        > MAX_GLB_BYTES as u64
    {
        bail!("GLB {} exceeds the 256 MiB preview limit", path.display());
    }
    // The take bound still applies if a working-tree file grows after metadata
    // was read; a large reported size takes the zero-allocation path above.
    let mut bytes = Vec::new();
    file.take(MAX_GLB_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| AssetError::read(path, source))?;
    validate_bytes(&bytes).with_context(|| format!("invalid GLB {}", path.display()))
}

/// Validates an in-memory GLB. Kept independent of assets so malformed fixtures
/// can exercise the same production checks without constructing a package.
fn validate_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_GLB_BYTES {
        bail!("GLB exceeds the 256 MiB preview limit");
    }
    let (document, binary) = parse_container(bytes)?;
    validate_asset(&document)?;
    validate_nodes(&document)?;
    let buffer_lengths = validate_buffers(&document, binary)?;
    let view_lengths = validate_buffer_views(&document, &buffer_lengths)?;
    validate_accessors(&document, &view_lengths)?;
    validate_meshes(&document)?;
    validate_images(&document, &view_lengths)?;
    validate_materials(&document)?;
    Ok(())
}

/// Decodes the GLB container without borrowing data after the byte slice ends.
fn parse_container(bytes: &[u8]) -> Result<(Value, Option<&[u8]>)> {
    if bytes.len() < 12 {
        bail!("file is shorter than the 12-byte GLB header");
    }
    if little_u32(bytes, 0)? != GLB_MAGIC {
        bail!("bad glb magic");
    }
    if little_u32(bytes, 4)? != 2 {
        bail!("unsupported GLB container version; expected 2");
    }
    let declared = little_u32(bytes, 8)? as usize;
    if declared != bytes.len() {
        bail!(
            "GLB header length {declared} does not match actual length {}",
            bytes.len()
        );
    }

    let mut offset = 12usize;
    let mut json = None;
    let mut binary = None;
    let mut chunk_index = 0usize;
    while offset < bytes.len() {
        if bytes.len() - offset < 8 {
            bail!("truncated GLB chunk header at byte {offset}");
        }
        let length = little_u32(bytes, offset)? as usize;
        if !length.is_multiple_of(4) {
            bail!("GLB chunk {chunk_index} length is not aligned to four bytes");
        }
        let kind = little_u32(bytes, offset + 4)?;
        let start = offset
            .checked_add(8)
            .ok_or_else(|| anyhow!("GLB chunk offset overflow"))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| anyhow!("GLB chunk length overflow"))?;
        if end > bytes.len() {
            bail!("GLB chunk {chunk_index} extends beyond the file");
        }
        match chunk_index {
            0 if kind != JSON_CHUNK => bail!("the first GLB chunk must be JSON"),
            0 => {
                if length > 16 * 1024 * 1024 {
                    bail!("GLB JSON chunk exceeds 16 MiB");
                }
                let text = std::str::from_utf8(&bytes[start..end])
                    .context("GLB JSON chunk is not UTF-8")?;
                json = Some(serde_json::from_str(text).context("invalid GLB JSON")?);
            }
            1 if kind == BIN_CHUNK => binary = Some(&bytes[start..end]),
            // Unknown chunks after the required JSON chunk are permitted by
            // glTF and intentionally ignored. A second BIN chunk is not.
            _ if kind == BIN_CHUNK => bail!("GLB contains more than one BIN chunk"),
            _ => {}
        }
        offset = end;
        chunk_index += 1;
    }
    Ok((
        json.ok_or_else(|| anyhow!("GLB has no JSON chunk"))?,
        binary,
    ))
}

fn validate_asset(document: &Value) -> Result<()> {
    let object = document
        .as_object()
        .ok_or_else(|| anyhow!("glTF root must be a JSON object"))?;
    let asset = object
        .get("asset")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("glTF asset object is required"))?;
    if asset.get("version").and_then(Value::as_str) != Some("2.0") {
        bail!("glTF asset.version must be exactly 2.0");
    }
    Ok(())
}

/// Checks scene references and the node hierarchy, including cycles and nodes
/// with multiple parents. The latter is also required by the OpenSDL component
/// resolver, which derives each component from one unambiguous node path.
fn validate_nodes(document: &Value) -> Result<()> {
    let nodes = array(document, "nodes")?;
    let scene_list = array(document, "scenes")?;
    for (index, scene) in scene_list.iter().enumerate() {
        let scene = object(scene, &format!("scenes[{index}]"))?;
        for root in optional_array(scene, "nodes", &format!("scenes[{index}]"))? {
            index_value(root, nodes.len(), &format!("scenes[{index}].nodes"))?;
        }
    }
    if let Some(scene) = document.get("scene") {
        index_value(scene, scene_list.len(), "default scene index")?;
    }

    let mut parents = vec![None; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let node = object(node, &format!("nodes[{index}]"))?;
        for child in optional_array(node, "children", &format!("nodes[{index}]"))? {
            let child = index_value(child, nodes.len(), &format!("nodes[{index}].children"))?;
            if parents[child].replace(index).is_some() {
                bail!("glTF node {child} has more than one parent");
            }
        }
        for key in ["mesh", "camera", "skin"] {
            if let Some(value) = node.get(key) {
                let length = array(document, key_plural(key))?.len();
                index_value(value, length, &format!("nodes[{index}].{key}"))?;
            }
        }
    }

    // One tri-colour traversal rejects cycles even outside the selected scene.
    // Completed nodes are never re-walked, keeping deep chains O(n), not O(n²).
    let mut marks = vec![0_u8; nodes.len()];
    for start in 0..nodes.len() {
        let mut path = Vec::new();
        let mut cursor = Some(start);
        while let Some(current) = cursor {
            match marks[current] {
                2 => break,
                1 => bail!("glTF node hierarchy contains a cycle at node {current}"),
                _ => {}
            }
            marks[current] = 1;
            path.push(current);
            cursor = parents[current];
        }
        for current in path {
            marks[current] = 2;
        }
    }
    Ok(())
}

/// Returns each declared buffer length. Every buffer must be backed by the one
/// standard BIN chunk, making the preview safe to fetch as a single artifact.
fn validate_buffers(document: &Value, binary: Option<&[u8]>) -> Result<Vec<usize>> {
    let buffers = array(document, "buffers")?;
    // glTF permits only the first buffer to omit `uri` and use the GLB BIN
    // chunk. Rejecting additional buffers also keeps every view's byte origin
    // unambiguous in this deliberately self-contained portable subset.
    if buffers.len() > 1 {
        bail!("portable GLB must contain exactly one BIN-backed buffer");
    }
    if !buffers.is_empty() && binary.is_none() {
        bail!("GLB buffers require a BIN chunk");
    }
    let binary = binary.unwrap_or_default();
    let mut lengths = Vec::with_capacity(buffers.len());
    for (index, buffer) in buffers.iter().enumerate() {
        let buffer = object(buffer, &format!("buffers[{index}]"))?;
        if buffer.contains_key("uri") {
            bail!("buffers[{index}].uri is not portable; embed bytes in the GLB BIN chunk");
        }
        let length = required_usize(buffer, "byteLength", &format!("buffers[{index}]"))?;
        if length > binary.len() {
            bail!(
                "buffers[{index}] declares {length} bytes but the BIN chunk has {}",
                binary.len()
            );
        }
        lengths.push(length);
    }
    Ok(lengths)
}

fn validate_buffer_views(document: &Value, buffers: &[usize]) -> Result<Vec<usize>> {
    let views = array(document, "bufferViews")?;
    let mut lengths = Vec::with_capacity(views.len());
    for (index, view) in views.iter().enumerate() {
        let view = object(view, &format!("bufferViews[{index}]"))?;
        let buffer = index_value(
            view.get("buffer")
                .ok_or_else(|| anyhow!("bufferViews[{index}].buffer is required"))?,
            buffers.len(),
            &format!("bufferViews[{index}].buffer"),
        )?;
        let length = required_usize(view, "byteLength", &format!("bufferViews[{index}]"))?;
        let offset = optional_usize(view, "byteOffset")?;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| anyhow!("bufferViews[{index}] range overflow"))?;
        if end > buffers[buffer] {
            bail!(
                "bufferViews[{index}] range 0..{end} exceeds buffer {buffer} length {}",
                buffers[buffer]
            );
        }
        lengths.push(length);
    }
    Ok(lengths)
}

/// Validates accessor addressing. Sparse accessors and implementation-defined
/// component encodings are rejected rather than silently receiving incomplete
/// bounds checks; the portable asset contract only needs dense glTF accessors.
fn validate_accessors(document: &Value, views: &[usize]) -> Result<()> {
    for (index, accessor) in array(document, "accessors")?.iter().enumerate() {
        let accessor = object(accessor, &format!("accessors[{index}]"))?;
        if accessor.contains_key("sparse") {
            bail!("accessors[{index}].sparse is outside the portable simulation subset");
        }
        let view = index_value(
            accessor
                .get("bufferView")
                .ok_or_else(|| anyhow!("accessors[{index}].bufferView is required"))?,
            views.len(),
            &format!("accessors[{index}].bufferView"),
        )?;
        let count = required_usize(accessor, "count", &format!("accessors[{index}]"))?;
        if count == 0 {
            bail!("accessors[{index}].count must be at least one");
        }
        let kind = accessor
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("accessors[{index}] has an invalid type"))?;
        let components = match kind {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" | "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            _ => bail!("accessors[{index}] has an invalid type"),
        };
        let component_bytes = match accessor.get("componentType").and_then(Value::as_u64) {
            Some(5120 | 5121) => 1,
            Some(5122 | 5123) => 2,
            Some(5125 | 5126) => 4,
            _ => bail!("accessors[{index}] has an invalid componentType"),
        };
        // MAT2/MAT3 with 8-/16-bit components pad each column to a four-byte
        // boundary. Their explicit stride includes the full padded matrix.
        let matrix_is_padded = component_bytes < 4 && matches!(kind, "MAT2" | "MAT3");
        let element_bytes = if matrix_is_padded {
            let columns: usize = if kind == "MAT2" { 2 } else { 3 };
            let column_bytes = (columns * component_bytes).next_multiple_of(4);
            columns * column_bytes
        } else {
            components * component_bytes
        };
        let view_object = document["bufferViews"][view]
            .as_object()
            .expect("buffer view was checked");
        let explicit_stride = view_object.contains_key("byteStride");
        if matrix_is_padded && !explicit_stride {
            bail!("accessors[{index}] padded matrices require an explicit byteStride");
        }
        let stride = if explicit_stride {
            optional_usize(view_object, "byteStride")?
        } else {
            element_bytes
        };
        if stride < element_bytes || stride > 252 {
            bail!("accessors[{index}] has an invalid byteStride {stride}");
        }
        if explicit_stride && !stride.is_multiple_of(4) {
            bail!("accessors[{index}].byteStride must be aligned to four bytes");
        }
        let offset = optional_usize(accessor, "byteOffset")?;
        let view_offset = optional_usize(view_object, "byteOffset")?;
        let absolute_offset = offset
            .checked_add(view_offset)
            .ok_or_else(|| anyhow!("accessors[{index}] offset overflow"))?;
        let alignment = if matrix_is_padded { 4 } else { component_bytes };
        if !absolute_offset.is_multiple_of(alignment) {
            bail!("accessors[{index}] byte offset is not component-aligned");
        }
        let payload = (count - 1)
            .checked_mul(stride)
            .and_then(|value| value.checked_add(element_bytes))
            .ok_or_else(|| anyhow!("accessors[{index}] range overflow"))?;
        let end = offset
            .checked_add(payload)
            .ok_or_else(|| anyhow!("accessors[{index}] range overflow"))?;
        if end > views[view] {
            bail!(
                "accessors[{index}] range 0..{end} exceeds bufferView {view} length {}",
                views[view]
            );
        }
    }
    Ok(())
}

fn validate_materials(document: &Value) -> Result<()> {
    let images = array(document, "images")?;
    let samplers = array(document, "samplers")?;
    let textures = array(document, "textures")?;
    for (index, texture) in textures.iter().enumerate() {
        let texture = object(texture, &format!("textures[{index}]"))?;
        if let Some(sampler) = texture.get("sampler") {
            index_value(
                sampler,
                samplers.len(),
                &format!("textures[{index}].sampler"),
            )?;
        }
        index_value(
            texture
                .get("source")
                .ok_or_else(|| anyhow!("textures[{index}].source is required"))?,
            images.len(),
            &format!("textures[{index}].source"),
        )?;
    }
    for (index, material) in array(document, "materials")?.iter().enumerate() {
        let material = object(material, &format!("materials[{index}]"))?;
        for key in ["normalTexture", "occlusionTexture", "emissiveTexture"] {
            if let Some(reference) = material.get(key) {
                validate_texture_reference(
                    reference,
                    textures.len(),
                    &format!("materials[{index}].{key}"),
                )?;
            }
        }
        if let Some(pbr) = material.get("pbrMetallicRoughness") {
            let pbr = object(pbr, &format!("materials[{index}].pbrMetallicRoughness"))?;
            for key in ["baseColorTexture", "metallicRoughnessTexture"] {
                if let Some(reference) = pbr.get(key) {
                    validate_texture_reference(
                        reference,
                        textures.len(),
                        &format!("materials[{index}].pbrMetallicRoughness.{key}"),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn validate_texture_reference(value: &Value, textures: usize, label: &str) -> Result<()> {
    let reference = object(value, label)?;
    index_value(
        reference
            .get("index")
            .ok_or_else(|| anyhow!("{label}.index is required"))?,
        textures,
        &format!("{label}.index"),
    )?;
    Ok(())
}

fn validate_meshes(document: &Value) -> Result<()> {
    let accessors = array(document, "accessors")?;
    let materials = array(document, "materials")?;
    for (mesh_index, mesh) in array(document, "meshes")?.iter().enumerate() {
        let mesh = object(mesh, &format!("meshes[{mesh_index}]"))?;
        let primitives = mesh
            .get("primitives")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| anyhow!("meshes[{mesh_index}].primitives must not be empty"))?;
        for (primitive_index, primitive) in primitives.iter().enumerate() {
            let label = format!("meshes[{mesh_index}].primitives[{primitive_index}]");
            let primitive = object(primitive, &label)?;
            let attributes = primitive
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("{label}.attributes must be an object"))?;
            if !attributes.contains_key("POSITION") {
                bail!("{label} has no POSITION accessor");
            }
            for (name, value) in attributes {
                index_value(
                    value,
                    accessors.len(),
                    &format!("{label}.attributes.{name}"),
                )?;
            }
            if let Some(indices) = primitive.get("indices") {
                index_value(indices, accessors.len(), &format!("{label}.indices"))?;
            }
            if let Some(material) = primitive.get("material") {
                index_value(material, materials.len(), &format!("{label}.material"))?;
            }
        }
    }
    Ok(())
}

fn validate_images(document: &Value, views: &[usize]) -> Result<()> {
    for (index, image) in array(document, "images")?.iter().enumerate() {
        let image = object(image, &format!("images[{index}]"))?;
        if image.contains_key("uri") {
            bail!("images[{index}].uri is not portable; use a GLB bufferView");
        }
        index_value(
            image
                .get("bufferView")
                .ok_or_else(|| anyhow!("images[{index}].bufferView is required"))?,
            views.len(),
            &format!("images[{index}].bufferView"),
        )?;
        if !matches!(
            image.get("mimeType").and_then(Value::as_str),
            Some("image/png" | "image/jpeg")
        ) {
            bail!("images[{index}].mimeType must be image/png or image/jpeg");
        }
    }
    Ok(())
}

fn key_plural(key: &str) -> &str {
    match key {
        "mesh" => "meshes",
        "camera" => "cameras",
        "skin" => "skins",
        _ => unreachable!("only indexed node fields are requested"),
    }
}

fn array<'a>(document: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
    static EMPTY: Vec<Value> = Vec::new();
    match document.get(key) {
        None => Ok(&EMPTY),
        Some(value) => value
            .as_array()
            .ok_or_else(|| anyhow!("glTF {key} must be an array")),
    }
}

fn optional_array<'a>(
    value: &'a serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<&'a Vec<Value>> {
    static EMPTY: Vec<Value> = Vec::new();
    match value.get(key) {
        None => Ok(&EMPTY),
        Some(value) => value
            .as_array()
            .ok_or_else(|| anyhow!("{label}.{key} must be an array")),
    }
}

fn object<'a>(value: &'a Value, label: &str) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| anyhow!("{label} must be an object"))
}

fn index_value(value: &Value, length: usize, label: &str) -> Result<usize> {
    let index = value
        .as_u64()
        .ok_or_else(|| anyhow!("{label} must be a non-negative integer"))? as usize;
    if index >= length {
        bail!("{label} index {index} is outside 0..{length}");
    }
    Ok(index)
}

fn required_usize(
    object: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<usize> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| anyhow!("{label}.{key} must be a non-negative integer"))
}

fn optional_usize(object: &serde_json::Map<String, Value>, key: &str) -> Result<usize> {
    match object.get(key) {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .map(|value| value as usize)
            .ok_or_else(|| anyhow!("{key} must be a non-negative integer")),
    }
}

fn little_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| anyhow!("truncated u32 at byte {offset}"))?
        .try_into()
        .expect("four-byte slice");
    Ok(u32::from_le_bytes(raw))
}

#[cfg(test)]
mod tests {
    use super::{validate, validate_bytes, MAX_GLB_BYTES};

    fn glb(json: &str, binary: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut binary = binary.to_vec();
        while !binary.len().is_multiple_of(4) {
            binary.push(0);
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&((28 + json.len() + binary.len()) as u32).to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0x004e_4942u32.to_le_bytes());
        bytes.extend_from_slice(&binary);
        bytes
    }

    #[test]
    fn rejects_an_out_of_bounds_accessor() {
        let bytes = glb(
            r##"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":4}],"accessors":[{"bufferView":0,"componentType":5126,"count":2,"type":"SCALAR"}]}"##,
            &[0; 4],
        );
        let error = validate_bytes(&bytes).unwrap_err().to_string();
        assert!(error.contains("accessors[0]"), "{error}");
    }

    #[test]
    fn rejects_external_buffer_dependencies() {
        let bytes = glb(
            r##"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4,"uri":"buffer.bin"}]}"##,
            &[0; 4],
        );
        let error = validate_bytes(&bytes).unwrap_err().to_string();
        assert!(error.contains("not portable"), "{error}");
    }

    #[test]
    fn rejects_multiple_uri_less_buffers_sharing_the_bin_chunk() {
        let bytes = glb(
            r##"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4},{"byteLength":4}]}"##,
            &[0; 8],
        );
        let error = validate_bytes(&bytes).unwrap_err().to_string();
        assert!(error.contains("exactly one BIN-backed buffer"), "{error}");
    }

    #[test]
    fn accepts_a_minimal_self_contained_mesh() {
        let bytes = glb(
            r##"{"asset":{"version":"2.0"},"buffers":[{"byteLength":12}],"bufferViews":[{"buffer":0,"byteLength":12}],"accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],"meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]}"##,
            &[0; 12],
        );
        validate_bytes(&bytes).unwrap();
    }

    #[test]
    fn rejects_non_array_hierarchy_fields() {
        for suffix in [
            r#","scenes":{}"#,
            r#","scenes":[{"nodes":{}}]"#,
            r#","nodes":[{"children":{}}]"#,
        ] {
            let json = format!(r#"{{"asset":{{"version":"2.0"}}{suffix}}}"#);
            assert!(validate_bytes(&glb(&json, &[])).is_err(), "{json}");
        }
    }

    #[test]
    fn rejects_unaligned_chunks() {
        let mut bytes = glb(r#"{"asset":{"version":"2.0"}}"#, &[]);
        let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        bytes[12..16].copy_from_slice(&(json_length + 1).to_le_bytes());
        let error = validate_bytes(&bytes).unwrap_err().to_string();
        assert!(error.contains("not aligned"), "{error}");
    }

    #[test]
    fn validates_padded_matrix_storage_and_alignment() {
        for (kind, component_type, size) in [
            ("MAT2", 5121, 8),
            ("MAT2", 5123, 8),
            ("MAT3", 5121, 12),
            ("MAT3", 5123, 24),
        ] {
            for view_size in [size - 1, size] {
                let json = format!(
                    r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":{size}}}],"bufferViews":[{{"buffer":0,"byteLength":{view_size},"byteStride":{size}}}],"accessors":[{{"bufferView":0,"componentType":{component_type},"count":1,"type":"{kind}"}}]}}"#
                );
                let result = validate_bytes(&glb(&json, &vec![0; size]));
                assert_eq!(result.is_ok(), view_size == size, "{kind}: {result:?}");
            }
        }

        let unaligned = r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteLength":4,"byteStride":2}],"accessors":[{"bufferView":0,"componentType":5123,"count":1,"type":"SCALAR"}]}"#;
        assert!(validate_bytes(&glb(unaligned, &[0; 4]))
            .unwrap_err()
            .to_string()
            .contains("four bytes"));
        let offset = r#"{"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteOffset":1,"byteLength":2}],"accessors":[{"bufferView":0,"componentType":5123,"count":1,"type":"SCALAR"}]}"#;
        assert!(validate_bytes(&glb(offset, &[0; 4]))
            .unwrap_err()
            .to_string()
            .contains("component-aligned"));
    }

    #[test]
    fn rejects_empty_accessors_and_invalid_material_references() {
        for suffix in [
            r#","buffers":[{"byteLength":1}],"bufferViews":[{"buffer":0,"byteLength":1}],"accessors":[{"bufferView":0,"componentType":5121,"count":0,"type":"SCALAR"}]"#,
            r#","materials":[null]"#,
            r#","textures":[{"source":0}]"#,
            r#","images":[{"bufferView":0}],"buffers":[{"byteLength":1}],"bufferViews":[{"buffer":0,"byteLength":1}]"#,
            r#","textures":[{}],"images":[null]"#,
            r#","materials":[{"normalTexture":{"index":0}}]"#,
        ] {
            let json = format!(r#"{{"asset":{{"version":"2.0"}}{suffix}}}"#);
            assert!(validate_bytes(&glb(&json, &[0; 4])).is_err(), "{json}");
        }
    }

    #[test]
    fn validates_a_deep_hierarchy_in_linear_time() {
        let mut nodes = String::new();
        for index in 0..4096 {
            if index > 0 {
                nodes.push(',');
            }
            nodes.push_str(&format!(r#"{{"children":[{}]}}"#, index + 1));
        }
        nodes.push_str(",{}");
        let json = format!(r#"{{"asset":{{"version":"2.0"}},"nodes":[{nodes}]}}"#);
        validate_bytes(&glb(&json, &[])).unwrap();
    }

    #[test]
    fn rejects_an_oversized_file_before_reading_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.glb");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_GLB_BYTES as u64 + 1).unwrap();
        let error = validate(&path).unwrap_err().to_string();
        assert!(error.contains("256 MiB"), "{error}");
    }
}
