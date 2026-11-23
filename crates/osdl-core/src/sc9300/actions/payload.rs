//! Main-controller command payloads. Fields follow the vendor's big-endian layouts.

use super::{Arguments, Axis};

use super::super::calibration::{liquid_pulses, xy_pulses, z_pulses};

const SPEED: u8 = 200;
const LOAD: [u8; 2] = [40, SPEED];
const MIX: [u8; 2] = [100, SPEED];

fn xy<const N: usize>(args: &Arguments<'_>, ramp: [u8; 2]) -> [u8; N] {
    let mut payload = [0; N];
    payload[0] = args.axis() as u8;
    payload[1..5].copy_from_slice(&xy_pulses(args.number("x", 0.0)).to_be_bytes());
    payload[5..7].copy_from_slice(&ramp);
    payload[7..11].copy_from_slice(&xy_pulses(args.number("y", 0.0)).to_be_bytes());
    payload[11..13].copy_from_slice(&ramp);
    payload
}

fn z(payload: &mut [u8], offset: usize, distance: f32) {
    payload[offset..offset + 4].copy_from_slice(&z_pulses(distance).to_be_bytes());
}

fn word(payload: &mut [u8], offset: usize, value: u16) {
    payload[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn dword(payload: &mut [u8], offset: usize, value: u32) {
    payload[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn move_to(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = xy::<20>(args, LOAD);
    z(&mut payload, 13, args.number("z", 0.0));
    payload[17..19].copy_from_slice(&LOAD);
    payload[19] = u8::from(args.text("method") == "together");
    payload.into()
}

pub(super) fn limit(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = [0; 17];
    payload[0] = args.axis() as u8;
    payload[1..5].copy_from_slice(&xy_pulses(args.number("x", 0.0)).to_be_bytes());
    payload[5..9].copy_from_slice(&xy_pulses(args.number("y", 0.0)).to_be_bytes());
    z(&mut payload, 9, args.number("z", 0.0));
    dword(&mut payload, 13, args.integer("plungerPulses", 0) as u32);
    payload.into()
}

pub(super) fn reset() -> Vec<u8> {
    let mut payload = [0; 12];
    z(&mut payload, 8, 20.0);
    payload.into()
}

pub(super) fn jog(args: &Arguments<'_>) -> Vec<u8> {
    let direction = match args.text("direction") {
        "x_left" => 0x01,
        "x_right" => 0x02,
        "y_forward" => 0x11,
        "y_backward" => 0x12,
        "z_left_up" => 0x21,
        "z_left_down" => 0x22,
        "z_right_up" => 0x23,
        "z_right_down" => 0x24,
        "gripper_up" => 0x25,
        "gripper_down" => 0x26,
        "left_plunger_aspirate" => 0x31,
        "left_plunger_dispense" => 0x32,
        "right_plunger_aspirate" => 0x33,
        "right_plunger_dispense" => 0x34,
        _ => unreachable!("validated jog direction"),
    };
    let distance = args.number("distanceMm", 0.0);
    let pulses = if direction < 0x20 {
        xy_pulses(distance)
    } else {
        z_pulses(distance)
    };
    jog_payload(direction, pulses)
}

pub(super) fn jaw(args: &Arguments<'_>) -> Vec<u8> {
    let direction = if args.text("action") == "release" {
        0x42
    } else {
        0x41
    };
    jog_payload(direction, args.integer("openingPulses", 0) as i32)
}

fn jog_payload(direction: u8, pulses: i32) -> Vec<u8> {
    let mut payload = [0; 5];
    payload[0] = direction;
    payload[1..].copy_from_slice(&pulses.to_be_bytes());
    payload.into()
}

pub(super) fn tips(args: &Arguments<'_>, pick_up: bool) -> Vec<u8> {
    let mut payload = xy::<31>(args, LOAD);
    let seat = args.number("zSeat", 0.0);
    let descent = if pick_up {
        args.number("zDescent", seat + 5.0)
    } else {
        seat + 5.0
    };
    z(&mut payload, 13, descent);
    payload[17..19].copy_from_slice(&LOAD);
    let rise = args.number("zUp", seat + 20.0);
    if pick_up {
        z(&mut payload, 19, seat);
        payload[23..25].copy_from_slice(&LOAD);
        z(&mut payload, 25, rise);
        payload[29..31].copy_from_slice(&LOAD);
        payload.into()
    } else {
        z(&mut payload, 19, rise);
        payload[23..25].copy_from_slice(&LOAD);
        payload[..25].to_vec()
    }
}

struct LiquidTarget {
    pulses: u32,
    bottom: f32,
    decline: f32,
    pause_ms: u16,
}

impl LiquidTarget {
    fn read(args: &Arguments<'_>) -> Result<Self, String> {
        let pulses = liquid_pulses(
            args.number("volumeUl", 0.0),
            args.integer("pipette", 0),
            args.text("mode"),
            args.axis(),
        )
        .ok_or("volume is outside the factory calibration for this head, pipette, and mode")?;
        let bottom = args.number("zBottom", 0.0);
        Ok(Self {
            pulses,
            bottom,
            decline: args.number("zDecline", bottom),
            pause_ms: args.integer("postPauseSeconds", 1) as u16 * 1000,
        })
    }
}

pub(super) fn aspirate(args: &Arguments<'_>) -> Result<Vec<u8>, String> {
    let target = LiquidTarget::read(args)?;
    let detect = u8::from(args.text("method") == "detect" || args.enabled("liquidLevelDetection"));
    let mut payload = xy::<85>(args, LOAD);
    z(&mut payload, 13, target.decline);
    z(&mut payload, 17, target.bottom);
    payload[25..27].copy_from_slice(&LOAD);
    z(&mut payload, 27, target.bottom + 1.0);
    payload[35] = detect;
    dword(&mut payload, 36, target.pulses);
    word(&mut payload, 40, SPEED.into());
    word(&mut payload, 42, target.pause_ms);
    z(&mut payload, 44, target.bottom + 6.0);
    payload[49] = SPEED;
    z(&mut payload, 56, target.bottom + 20.0);
    payload[60..62].copy_from_slice(&LOAD);
    word(&mut payload, 62, LOAD[0].into());
    payload[73] = detect;
    payload[74] = SPEED;
    z(&mut payload, 75, target.bottom);
    z(&mut payload, 79, target.bottom);
    Ok(payload.into())
}

pub(super) fn dispense(args: &Arguments<'_>) -> Result<Vec<u8>, String> {
    let target = LiquidTarget::read(args)?;
    let mut payload = xy::<83>(args, LOAD);
    z(&mut payload, 13, target.decline);
    z(&mut payload, 17, target.decline);
    z(&mut payload, 25, target.bottom);
    payload[29..33].copy_from_slice(&xy_pulses(args.number("x", 0.0)).to_be_bytes());
    payload[33..35].copy_from_slice(&LOAD);
    payload[35] = match args.text("method") {
        "bottle_neck" => 1,
        "offset" => 2,
        _ => 0,
    };
    dword(&mut payload, 36, target.pulses);
    word(&mut payload, 40, SPEED.into());
    word(&mut payload, 42, target.pause_ms);
    if args.enabled("blowOut") {
        dword(&mut payload, 44, target.pulses / 10);
    }
    word(&mut payload, 48, SPEED.into());
    z(&mut payload, 50, target.bottom + 6.0);
    word(&mut payload, 54, SPEED.into());
    z(&mut payload, 62, target.bottom + 20.0);
    payload[66..68].copy_from_slice(&LOAD);
    word(&mut payload, 68, LOAD[0].into());
    word(&mut payload, 70, LOAD[0].into());
    payload[72..76].copy_from_slice(&xy_pulses(args.number("y", 0.0)).to_be_bytes());
    payload[78..81].copy_from_slice(&[args.integer("mixCount", 0) as u8, SPEED, MIX[0]]);
    word(&mut payload, 81, 1000);
    Ok(payload.into())
}

pub(super) fn homogenize(args: &Arguments<'_>) -> Result<Vec<u8>, String> {
    let target = LiquidTarget::read(args)?;
    let mut payload = xy::<82>(args, MIX);
    z(&mut payload, 13, target.decline);
    z(&mut payload, 17, target.decline);
    payload[25..27].copy_from_slice(&MIX);
    z(&mut payload, 27, target.bottom + 1.0);
    dword(&mut payload, 35, target.pulses);
    payload[39] = args.integer("mixCount", 0) as u8;
    word(&mut payload, 40, SPEED.into());
    word(&mut payload, 42, SPEED.into());
    z(&mut payload, 60, target.bottom);
    payload[64..66].copy_from_slice(&MIX);
    word(&mut payload, 66, target.pause_ms);
    z(&mut payload, 68, target.bottom + 20.0);
    payload[72..74].copy_from_slice(&MIX);
    word(&mut payload, 74, MIX[0].into());
    word(&mut payload, 76, MIX[0].into());
    Ok(payload.into())
}

pub(super) fn clamp(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = [0; 44];
    let height = args.number("zClamp", 0.0);
    payload[0] = Axis::Gripper as u8;
    payload[1..5].copy_from_slice(&xy_pulses(args.number("xClamp", 0.0)).to_be_bytes());
    payload[5..9].copy_from_slice(&xy_pulses(args.number("yClamp", 0.0)).to_be_bytes());
    z(&mut payload, 9, args.number("zApproach", height));
    z(&mut payload, 13, height);
    payload[17..21].copy_from_slice(&xy_pulses(args.number("xPlace", 0.0)).to_be_bytes());
    payload[21..25].copy_from_slice(&xy_pulses(args.number("yPlace", 0.0)).to_be_bytes());
    z(&mut payload, 25, args.number("zPlace", height));
    z(&mut payload, 29, height);
    word(&mut payload, 33, args.integer("jawOpening", 0) as u16);
    payload[35..43].copy_from_slice(&[MIX[0], SPEED, MIX[0], SPEED, MIX[0], SPEED, MIX[0], SPEED]);
    payload[43] = args.integer("force", 3) as u8;
    payload.into()
}

fn tenths(value: f32) -> u16 {
    (f64::from(value) * 10.0).round() as i16 as u16
}

pub(super) fn temperature(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = [0; 6];
    word(&mut payload, 1, args.integer("seconds", 0) as u16);
    word(&mut payload, 3, tenths(args.number("celsius", 0.0)));
    payload[5] = args.integer("module", 0) as u8;
    payload.into()
}

pub(super) fn oscillate(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = [0; 14];
    word(&mut payload, 9, args.integer("seconds", 0) as u16);
    word(&mut payload, 11, args.integer("rpm", 0) as u16);
    payload[13] = args.integer("module", 0) as u8;
    payload.into()
}

pub(super) fn temperature_and_oscillate(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = [0; 19];
    let seconds = args.integer("seconds", 0) as u16;
    word(&mut payload, 1, seconds);
    word(&mut payload, 3, tenths(args.number("celsius", 0.0)));
    word(&mut payload, 14, seconds);
    word(&mut payload, 16, args.integer("rpm", 0) as u16);
    payload[18] = args.integer("module", 0) as u8;
    payload.into()
}

pub(super) fn magnetic_stand(args: &Arguments<'_>) -> Vec<u8> {
    let mut payload = [0; 5];
    word(&mut payload, 0, args.integer("seconds", 0) as u16);
    word(&mut payload, 2, tenths(args.number("heightMm", 0.0)));
    payload[4] = args.integer("module", 0) as u8;
    payload.into()
}

pub(super) fn cabinet(args: &Arguments<'_>, on_mode: u8, off_mode: u8) -> Vec<u8> {
    let mut payload = [0; 7];
    payload[0] = if args.enabled("on") {
        on_mode
    } else {
        off_mode
    };
    word(&mut payload, 1, args.integer("seconds", 0) as u16);
    dword(&mut payload, 3, args.integer("rpm", 0) as u32);
    payload.into()
}
