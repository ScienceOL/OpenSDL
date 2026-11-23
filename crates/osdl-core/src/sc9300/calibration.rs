use super::actions::Axis;

const STEPS_PER_TURN: f32 = 51_200.0;
pub(super) const XY_PITCH_MM: f32 = 60.0;
pub(super) const Z_PITCH_MM: f32 = 12.7;

fn motor_pulses(distance_mm: f32, pitch_mm: f32) -> i32 {
    (distance_mm * STEPS_PER_TURN / pitch_mm).floor() as i32
}

pub(super) fn xy_pulses(distance_mm: f32) -> i32 {
    motor_pulses(distance_mm, XY_PITCH_MM)
}

pub(super) fn z_pulses(distance_mm: f32) -> i32 {
    motor_pulses(distance_mm, Z_PITCH_MM)
}

pub(super) fn millimetres(pulses: i32, pitch_mm: f32) -> f64 {
    (f64::from(pulses as f32 * pitch_mm / STEPS_PER_TURN) * 100.0).round() / 100.0
}

// Each factory segment holds inclusive microlitre bounds and the linear
// coefficients used to convert a volume to plunger pulses.
type Segment = (f32, f32, f32, f32);

pub(super) fn liquid_pulses(volume: f32, pipette: u64, mode: &str, axis: Axis) -> Option<u32> {
    let segments: &[Segment] = match (pipette, mode, axis) {
        (10, _, _) => &[
            (0.1, 2.0, 62616.0, 106.49),
            (2.0, 7.0, 52421.0, 20977.0),
            (7.0, 11.0, 51942.0, 21434.0),
        ],
        (20, "single", _) => &[
            (0.1, 11.0, 29963.0, 49552.0),
            (11.0, 21.0, 33610.0, 17115.0),
        ],
        (20, _, _) => &[
            (0.1, 11.0, 29963.0, 49552.0),
            (11.0, 25.0, 33610.0, 17115.0),
        ],
        (50, _, _) => &[
            (0.1, 25.0, 20211.0, 10779.0),
            (25.0, 55.0, 20015.0, 17507.0),
        ],
        (300, _, _) => &[
            (1.0, 5.0, 1981.1, 3498.1),
            (5.0, 50.0, 2113.3, 2810.8),
            (50.0, 150.0, 2093.7, 2960.2),
            (150.0, 350.0, 2085.0, 3548.3),
        ],
        (1000, "single", Axis::Left) => &[
            (1.0, 330.0, 2229.6, 3082.7),
            (330.0, 400.0, 2156.6, 9582.1),
            (400.0, 501.0, 2087.9, 37256.0),
            (501.0, 600.0, 2185.0, -12375.0),
            (600.0, 700.0, 2222.0, 30370.0),
            (700.0, 800.0, 1705.0, 323336.0),
            (800.0, 900.0, 2068.0, 61331.0),
            (900.0, 1001.0, 2047.2, 78418.0),
        ],
        (1000, "six" | "twelve", _) => &[
            (1.0, 100.0, 964.19, 1207.7),
            (100.0, 500.0, 937.8, 3550.1),
            (500.0, 1050.0, 928.69, 8253.7),
        ],
        _ => return None,
    };
    segments
        .iter()
        .find(|&&(start, end, _, _)| (start..=end).contains(&volume))
        .map(|&(_, _, slope, offset)| (slope * volume + offset).round() as u32)
}
