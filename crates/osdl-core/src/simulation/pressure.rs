//! Canonical isothermal fixed-volume pump/leak solution, kPa, litres and seconds.
pub(super) fn pressure_after(
    current: f64,
    dt: f64,
    pump: f64,
    leak: f64,
    ultimate: f64,
    ambient: f64,
    volume: f64,
) -> f64 {
    let equilibrium = (pump * ultimate + leak * ambient) / (pump + leak);
    let next = equilibrium + (current - equilibrium) * (-(pump + leak) * dt / volume).exp();
    if (next - equilibrium).abs() < 1e-7 {
        equilibrium
    } else {
        next
    }
}
