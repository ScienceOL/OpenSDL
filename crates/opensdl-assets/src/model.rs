//! Portable instrument metadata is admitted with the GLB, before any runtime starts.
use crate::AssetError;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDefinition {
    pub schema_version: u32,
    pub model_id: String,
    pub dimensions_m: [f64; 3],
    pub components: Vec<Component>,
    pub joints: Vec<Joint>,
    pub readouts: Vec<Readout>,
    pub visibility: Vec<Visibility>,
    pub placements: Vec<Placement>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: String,
    pub label: String,
    pub parent: String,
    pub control: Option<Control>,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Command {
        action: String,
        params: HashMap<String, Value>,
    },
    Toggle {
        action: String,
        params: HashMap<String, Value>,
        parameter: String,
        state: String,
        open: f64,
        closed: f64,
    },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Joint {
    pub id: String,
    pub component: String,
    pub kind: JointKind,
    pub axis: [f64; 3],
    pub min: f64,
    pub max: f64,
    pub state: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    Prismatic,
    Revolute,
    Continuous,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Readout {
    pub component: String,
    pub state: String,
    pub digits: u8,
    pub unit: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Visibility {
    pub component: String,
    pub state: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub id: String,
    pub component: String,
    pub position: [f64; 3],
    pub radius_m: f64,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}
fn invalid(message: &str) -> AssetError {
    AssetError::Invalid(format!("instrument model: {message}"))
}

impl ModelDefinition {
    pub fn validate(&self) -> Result<(), AssetError> {
        if self.schema_version != 1
            || !valid_id(&self.model_id)
            || self
                .dimensions_m
                .iter()
                .any(|n| !n.is_finite() || *n <= 0.0 || *n >= 100.0)
            || [
                self.components.len(),
                self.joints.len(),
                self.readouts.len(),
                self.visibility.len(),
                self.placements.len(),
            ]
            .iter()
            .any(|n| *n > 256)
        {
            return Err(invalid("identity, version, dimensions or structure limit"));
        }
        let mut components = HashMap::new();
        for c in &self.components {
            if !valid_id(&c.id)
                || c.id == "instrument"
                || !valid_id(&c.parent)
                || c.label.len() > 512
                || components.insert(c.id.as_str(), c).is_some()
            {
                return Err(invalid("component identity"));
            }
            if let Some(control) = &c.control {
                let (action, params) = match control {
                    Control::Command { action, params } => (action, params),
                    Control::Toggle {
                        action,
                        params,
                        parameter,
                        state,
                        open,
                        closed,
                    } => {
                        if !valid_id(parameter)
                            || !valid_id(state)
                            || !open.is_finite()
                            || !closed.is_finite()
                        {
                            return Err(invalid("toggle control"));
                        }
                        (action, params)
                    }
                };
                if !valid_id(action)
                    || params.len() > 16
                    || params.iter().any(|(k, v)| {
                        !valid_id(k)
                            || !(v.is_number()
                                || v.is_boolean()
                                || v.as_str().is_some_and(|s| s.len() < 512))
                    })
                {
                    return Err(invalid("action parameters"));
                }
            }
        }
        for c in &self.components {
            let mut seen = HashSet::from([c.id.as_str()]);
            let mut parent = c.parent.as_str();
            while parent != "instrument" {
                if !seen.insert(parent) {
                    return Err(invalid("component cycle"));
                }
                parent = components
                    .get(parent)
                    .ok_or_else(|| invalid("missing parent"))?
                    .parent
                    .as_str();
            }
        }
        let mut ids = HashSet::new();
        let mut moving = HashSet::new();
        for j in &self.joints {
            let norm = j.axis.iter().map(|n| n * n).sum::<f64>().sqrt();
            if !valid_id(&j.id)
                || !ids.insert(&j.id)
                || !components.contains_key(j.component.as_str())
                || !moving.insert(&j.component)
                || j.axis.iter().any(|n| !n.is_finite())
                || (norm - 1.0).abs() > 1e-6
                || !j.min.is_finite()
                || !j.max.is_finite()
                || j.min > j.max
                || !valid_id(&j.state)
            {
                return Err(invalid("joint frame or binding"));
            }
        }
        for (component, state) in self
            .readouts
            .iter()
            .map(|r| (&r.component, &r.state))
            .chain(self.visibility.iter().map(|v| (&v.component, &v.state)))
        {
            if !components.contains_key(component.as_str()) || !valid_id(state) {
                return Err(invalid("display/visibility binding"));
            }
        }
        if self
            .readouts
            .iter()
            .any(|r| r.digits > 8 || r.unit.len() > 32)
        {
            return Err(invalid("readout precision"));
        }
        for p in &self.placements {
            if !valid_id(&p.id)
                || !components.contains_key(p.component.as_str())
                || !p.radius_m.is_finite()
                || p.radius_m <= 0.0
                || p.position.iter().any(|n| !n.is_finite())
            {
                return Err(invalid("placement frame"));
            }
        }
        Ok(())
    }
}

pub fn read_preview_model(path: &Path) -> Result<Option<ModelDefinition>, AssetError> {
    let mut file = File::open(path).map_err(|e| AssetError::Read {
        path: path.to_owned(),
        source: e,
    })?;
    let mut header = [0u8; 20];
    file.read_exact(&mut header).map_err(|e| AssetError::Read {
        path: path.to_owned(),
        source: e,
    })?;
    let length = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if &header[..4] != b"glTF" || &header[16..20] != b"JSON" || length > 16 * 1024 * 1024 {
        return Err(invalid("GLB JSON chunk"));
    }
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes).map_err(|e| AssetError::Read {
        path: path.to_owned(),
        source: e,
    })?;
    let json: Value =
        serde_json::from_slice(&bytes).map_err(|e| invalid(&format!("GLB metadata: {e}")))?;
    let Some(nodes) = json.get("nodes").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut declaration = None;
    let mut parts = HashMap::new();
    let mut parents = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                let child = child.as_u64().ok_or_else(|| invalid("node child"))? as usize;
                if child >= nodes.len() || parents.insert(child, index).is_some() {
                    return Err(invalid("node parent"));
                }
            }
        }
        if let Some(extra) = node.get("extras") {
            if let Some(id) = extra.get("opensdlComponent").and_then(Value::as_str) {
                if !valid_id(id) || parts.insert(id, index).is_some() {
                    return Err(invalid("duplicate component node"));
                }
            }
            if let Some(model) = extra.get("opensdlModel") {
                if declaration.replace(model).is_some() {
                    return Err(invalid("multiple model roots"));
                }
            }
        }
    }
    let Some(value) = declaration else {
        return Ok(None);
    };
    if serde_json::to_vec(value)
        .map_err(|_| invalid("model JSON"))?
        .len()
        > 48 * 1024
    {
        return Err(invalid("metadata exceeds 48 KiB"));
    }
    let model: ModelDefinition =
        serde_json::from_value(value.clone()).map_err(|e| invalid(&e.to_string()))?;
    model.validate()?;
    for c in &model.components {
        let child = parts
            .get(c.id.as_str())
            .ok_or_else(|| invalid("missing component node"))?;
        let parent = parts
            .get(c.parent.as_str())
            .ok_or_else(|| invalid("missing component parent node"))?;
        if parents.get(child) != Some(parent) {
            return Err(invalid("node hierarchy differs from model structure"));
        }
    }
    Ok(Some(model))
}
