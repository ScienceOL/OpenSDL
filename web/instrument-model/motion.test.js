import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { motionDistance } from "./motion.js";
import { parseModelState } from "./contract.js";

const fixtures = JSON.parse(
  await readFile(new URL("./motion-fixtures.json", import.meta.url)),
);
for (const f of fixtures) {
  test(`native/client analytic trajectory: ${f.name}`, () => {
    const scale = Math.PI / 30;
    const motion = {
      target_velocity: f.target_rpm * scale * f.ratio,
      slew_per_s: f.slew_rpm_s * scale * Math.abs(f.ratio),
      stop_after_s: f.stop_after_s,
    };
    const radians = motionDistance(f.rpm * scale * f.ratio, motion, f.seconds);
    assert.ok(Math.abs(radians - f.expected_radians) < 1e-10);
  });
}

test("motion observations reject absent, nested, unbound or unbounded trajectories", () => {
  const state = {
    schema_version: 2,
    model_id: "motor",
    run_id: "one",
    revision: 0,
    time_s: 0,
    values: { speed: 1 },
    motion: {
      speed: { target_velocity: 0, slew_per_s: 2, stop_after_s: null },
    },
  };
  assert.ok(parseModelState(state));
  for (const profile of [
    { target_velocity: 0, slew_per_s: 0, stop_after_s: null },
    { target_velocity: Infinity, slew_per_s: 2, stop_after_s: null },
    { target_velocity: 0, slew_per_s: 2, stop_after_s: -1 },
    { target_velocity: 0, slew_per_s: 2 },
    { target_velocity: 0, slew_per_s: 2, stop_after_s: null, path: "private" },
  ])
    assert.equal(
      parseModelState({ ...state, motion: { speed: profile } }),
      null,
    );
  assert.equal(parseModelState({ ...state, motion: undefined }), null);
  assert.equal(parseModelState({ ...state, values: {} }), null);
  assert.equal(parseModelState({ ...state, schema_version: 1 }), null);
});
