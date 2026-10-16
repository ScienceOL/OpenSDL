//! Lumped thermal nodes; energies are joules, conductances W/K, time seconds.
#[derive(Debug, Clone)]
pub struct ThermalPair {
    pub plate_c: f64,
    pub liquid_c: f64,
    pub input_j: f64,
    pub loss_j: f64,
}
impl ThermalPair {
    pub fn new(ambient: f64) -> Self {
        Self {
            plate_c: ambient,
            liquid_c: ambient,
            input_j: 0.0,
            loss_j: 0.0,
        }
    }
    /// Fixed small substeps keep the energy balance exact for each exchanged packet.
    pub fn advance(
        &mut self,
        dt: f64,
        power: impl Fn(f64) -> f64,
        capacities: [f64; 2],
        conductances: [f64; 3],
        ambient: f64,
        loaded: bool,
    ) {
        let steps = (dt / 0.02).ceil().max(1.0) as u32;
        let h = dt / steps as f64;
        for _ in 0..steps {
            let power_w = power(self.plate_c);
            let exchange = if loaded {
                conductances[0] * (self.plate_c - self.liquid_c) * h
            } else {
                0.0
            };
            let plate_loss = conductances[1] * (self.plate_c - ambient) * h;
            let liquid_loss = if loaded {
                conductances[2] * (self.liquid_c - ambient) * h
            } else {
                0.0
            };
            self.plate_c += (power_w * h - exchange - plate_loss) / capacities[0];
            if loaded {
                self.liquid_c += (exchange - liquid_loss) / capacities[1];
            }
            self.input_j += power_w * h;
            self.loss_j += plate_loss + liquid_loss;
        }
    }
}
pub fn approach(current: f64, target: f64, rate: f64, dt: f64) -> f64 {
    current + (target - current).clamp(-rate * dt, rate * dt)
}
pub fn empty(value: &serde_json::Value) -> Result<(), String> {
    if value.as_object().is_some_and(|v| v.is_empty()) {
        Ok(())
    } else {
        Err("INVALID_PARAMS".into())
    }
}
pub fn number(value: &serde_json::Value, key: &str, min: f64, max: f64) -> Result<f64, String> {
    let object = value.as_object().ok_or("INVALID_PARAMS")?;
    if object.len() != 1 {
        return Err("INVALID_PARAMS".into());
    }
    object
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|n| n.is_finite() && (min..=max).contains(n))
        .ok_or("PARAMETER_LIMIT".into())
}
