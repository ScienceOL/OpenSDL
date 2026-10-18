/** Exact signed ramp integral. Prediction never changes the observed readouts. */
function ramp(velocity, target, slew, seconds) {
  const duration = Math.min(Math.abs(target - velocity) / slew, seconds);
  return (
    velocity * duration +
    Math.sign(target - velocity) * slew * duration * duration * 0.5 +
    target * (seconds - duration)
  );
}

export function motionDistance(velocity, motion, seconds) {
  const {
    target_velocity: target,
    slew_per_s: slew,
    stop_after_s: stop,
  } = motion;
  if (stop === null || stop >= seconds)
    return ramp(velocity, target, slew, seconds);
  const atStop =
    velocity +
    Math.sign(target - velocity) *
      Math.min(Math.abs(target - velocity), slew * stop);
  return (
    ramp(velocity, target, slew, stop) + ramp(atStop, 0, slew, seconds - stop)
  );
}
