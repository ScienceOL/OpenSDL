//! Structural validation for the self-contained glTF 2.0 binary payload.
//!
//! The OpenSDL declarations in `extras` describe presentation state; they do
//! not make a malformed glTF asset valid. This module therefore validates the
//! container, JSON object graph, embedded resources, and accessor bounds before
//! the presentation declarations are inspected. Portable previews deliberately
//! use only the standard GLB BIN chunk: external files and data URIs would make
//! the independently distributed `preview.glb` incomplete.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::AssetError;

const GLB_MAGIC: u32 = 0x4654_6c67;
const JSON_CHUNK: u32 = 0x4e4f_534a;
const BIN_CHUNK: u32 = 0x004e_4942;

/// Reads and validates a self-contained glTF 2.0 binary file.
///
/// In addition to the container and JSON graph, this checks scene/node indices,
/// mesh accessor references, buffer-view ranges, and accessor strides. It does
/// not claim to be a replacement for the complete Khronos glTF validator: GPU
/// material semantics and renderer-specific extensions remain renderer checks.
pub(crate) fn validate(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).map_err(|source| AssetError::read(path, source))?;
    validate_bytes(&bytes).with_context(|| format!("invalid GLB {}", path.display()))
}

/// Validates an in-memory GLB. Kept independent of assets so malformed fixtures
/// can exercise the same production checks without constructing a package.
fn validate_bytes(bytes: &[u8]) -> Result<()> {
    let (document, binary) = parse_container(bytes)?;
    validate_asset(&document)?;
    validate_nodes(&document)?;
    let buffer_lengths = validate_buffers(&document, binary)?;
    let view_lengths = validate_buffer_views(&document, &buffer_lengths)?;
    validate_accessors(&document, &view_lengths)?;
    validate_meshes(&document)?;
    validate_images(&document, &view_lengths)?;
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
    let empty = Vec::new();
    let scene_list = document
        .get("scenes")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    for (index, scene) in scene_list.iter().enumerate() {
        let scene = object(scene, &format!("scenes[{index}]"))?;
        for root in scene
            .get("nodes")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
        {
            index_value(root, nodes.len(), &format!("scenes[{index}].nodes"))?;
        }
    }
    if let Some(scene) = document.get("scene") {
        index_value(scene, scene_list.len(), "default scene index")?;
    }

    let mut parents = vec![None; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let node = object(node, &format!("nodes[{index}]"))?;
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
        {
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

    // Scene reachability is validated separately; checking every parent chain
    // also rejects cycles in nodes omitted from the selected scene.
    for node in 0..nodes.len() {
        let mut cursor = Some(node);
        let mut visited = HashSet::new();
        while let Some(current) = cursor {
            if !visited.insert(current) {
                bail!("glTF node hierarchy contains a cycle at node {current}");
            }
            cursor = parents[current];
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
        let components = match accessor.get("type").and_then(Value::as_str) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") | Some("MAT2") => 4,
            Some("MAT3") => 9,
            Some("MAT4") => 16,
            _ => bail!("accessors[{index}] has an invalid type"),
        };
        let component_bytes = match accessor.get("componentType").and_then(Value::as_u64) {
            Some(5120 | 5121) => 1,
            Some(5122 | 5123) => 2,
            Some(5125 | 5126) => 4,
            _ => bail!("accessors[{index}] has an invalid componentType"),
        };
        let element_bytes = components * component_bytes;
        let view_object = document["bufferViews"][view]
            .as_object()
            .expect("buffer view was checked");
        let stride = if view_object.contains_key("byteStride") {
            optional_usize(view_object, "byteStride")?
        } else {
            element_bytes
        };
        if stride < element_bytes || stride > 252 || stride % component_bytes != 0 {
            bail!("accessors[{index}] has an invalid byteStride {stride}");
        }
        let offset = optional_usize(accessor, "byteOffset")?;
        let payload = if count == 0 {
            0
        } else {
            (count - 1)
                .checked_mul(stride)
                .and_then(|value| value.checked_add(element_bytes))
                .ok_or_else(|| anyhow!("accessors[{index}] range overflow"))?
        };
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
    use super::validate_bytes;

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
}
