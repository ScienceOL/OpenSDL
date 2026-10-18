import test from "node:test";
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { createHash } from "node:crypto";
import { Box3, Vector3 } from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { bindInstrument } from "./index.js";
const folder = new URL("../../assets/instruments/", import.meta.url);
async function load(slug) {
  const bytes = await readFile(new URL(`${slug}/package/preview.glb`, folder));
  return (
    await new GLTFLoader().parseAsync(
      bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
      "",
    )
  ).scene;
}
function observation(binding) {
  const values = {};
  for (const j of binding.definition.joints) {
    values[j.state] = j.min;
    if (j.velocity_state) values[j.velocity_state] = 0;
  }
  for (const c of binding.definition.components)
    if (c.scale) values[c.scale.state] = c.scale.min;
  for (const r of binding.definition.readouts) values[r.state] = null;
  for (const b of binding.definition.visibility) values[b.state] = false;
  return {
    schema_version: 1,
    model_id: binding.definition.model_id,
    run_id: "catalog-test",
    revision: 0,
    time_s: 0,
    values,
  };
}
function meshVolume(part) {
  let result = 0;
  part.traverse((node) => {
    if (!node.isMesh) return;
    const p = node.geometry.attributes.position,
      indices = node.geometry.index;
    for (let i = 0; i < (indices?.count ?? p.count); i += 3) {
      const vertices = [0, 1, 2].map((k) =>
        new Vector3()
          .fromBufferAttribute(p, indices ? indices.getX(i + k) : i + k)
          .applyMatrix4(node.matrixWorld),
      );
      result += vertices[0].dot(vertices[1].clone().cross(vertices[2])) / 6;
    }
  });
  return Math.abs(result);
}

for (const slug of (await readdir(folder, { withFileTypes: true }))
  .filter((d) => d.isDirectory())
  .map((d) => d.name)) {
  test(`${slug}: actual geometry, closed bindings and current source provenance`, async () => {
    const scene = await load(slug),
      binding = bindInstrument(scene);
    assert.ok(binding);
    const size = new Box3().setFromObject(scene).getSize(new Vector3());
    binding.definition.dimensions_m.forEach((expected, i) =>
      assert.ok(
        Math.abs(size.getComponent(i) - expected) <= expected * 0.005 + 0.00001,
        `${slug} axis ${i}: ${size.getComponent(i)}`,
      ),
    );
    const state = observation(binding);
    binding.apply(state);
    for (const j of binding.definition.joints) {
      binding.inspect(j.id, j.min);
      binding.inspect(j.id, (j.min + j.max) / 2);
      binding.inspect(j.id, j.max);
    }
    const p = JSON.parse(
      await readFile(new URL(`${slug}/package/provenance.json`, folder)),
    );
    for (const [path, digest] of Object.entries(p.inputs))
      assert.equal(
        createHash("sha256")
          .update(await readFile(new URL("../../" + path, import.meta.url)))
          .digest("hex"),
        digest,
        path,
      );
    const manifest = JSON.parse(
      await readFile(new URL(`${slug}/package/asset.yaml`, folder)),
    );
    assert.equal(manifest.metadata.license.spdx, "CC-BY-NC-4.0");
    assert.match(
      await readFile(new URL(`${slug}/package/LICENSE`, folder), "utf8"),
      /Attribution-NonCommercial 4.0/,
    );
  });
}
test("dial uses confirmed values, clamps both directions, and rotor rendering expires after transport silence", async () => {
  const scene = await load("heated-stirrer"),
    a = bindInstrument(scene),
    b = bindInstrument(scene.clone(true));
  const state = observation(a);
  Object.assign(state.values, {
    target_rpm: 600,
    target_c: 80,
    bar_rad_s: Math.PI * 20,
  });
  a.apply(state);
  b.apply({ ...state, values: { ...state.values, bar_rad_s: 0 } });
  const dial = a.parts.get("speed_dial").children[0];
  assert.deepEqual(a.action(dial, state.values, 2), {
    action: "set_speed",
    params: { rpm: 800 },
  });
  assert.deepEqual(a.action(dial, state.values, -100), {
    action: "set_speed",
    params: { rpm: 0 },
  });
  assert.deepEqual(a.action(dial, state.values, 100), {
    action: "set_speed",
    params: { rpm: 1500 },
  });
  a.animate(0.025);
  assert.ok(Math.abs(a.parts.get("bar").quaternion.y) > 0.5);
  assert.equal(b.parts.get("bar").quaternion.y, 0);
  a.animate(0.3);
  const frozen = a.parts.get("bar").quaternion.clone();
  a.animate(1);
  assert.ok(frozen.equals(a.parts.get("bar").quaternion));
});

test("vortex cup follows its eccentric orbit without axial spin", async () => {
  const { Quaternion } = await import("three");
  const root = await load("vortex-mixer"),
    binding = bindInstrument(root),
    state = observation(binding);
  Object.assign(state.values, {
    orbit_rad: 0,
    cup_rad: 0,
    orbit_rad_s: 0,
    cup_rad_s: 0,
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  const cup = binding.parts.get("cup"),
    centre = binding.parts.get("eccentric");
  const rest = cup.getWorldQuaternion(new Quaternion());
  const first = cup.getWorldPosition(new Vector3());
  Object.assign(state.values, {
    orbit_rad: Math.PI / 2,
    cup_rad: -Math.PI / 2,
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  assert.ok(
    Math.abs(cup.getWorldQuaternion(new Quaternion()).dot(rest)) > 1 - 1e-8,
  );
  assert.ok(
    Math.abs(
      cup
        .getWorldPosition(new Vector3())
        .distanceTo(centre.getWorldPosition(new Vector3())) - 0.002,
    ) < 1e-7,
  );
  assert.ok(cup.getWorldPosition(new Vector3()).distanceTo(first) > 0.002);
});

test("pipette liquid volume, fixture positions, and clockwise encoders follow the actual mesh", async () => {
  const root = await load("pipette"),
    binding = bindInstrument(root),
    state = observation(binding);
  Object.assign(state.values, {
    pipette_x_m: -0.05,
    pipette_vertical_m: -0.0115,
    tip_visible: true,
    liquid_visible: true,
    fill_fraction: 0.5,
    source_fraction: 0.95,
    receiver_fraction: 0,
    source_liquid_visible: true,
    receiver_liquid_visible: false,
    volume_ul: 100,
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  const tip = new Box3().setFromObject(binding.parts.get("tip"));
  assert.ok(Math.abs(tip.getCenter(new Vector3()).x + 0.05) < 1e-6);
  assert.ok(Math.abs(tip.min.y - 0.0105) < 1e-6);
  assert.ok(
    Math.abs(meshVolume(binding.parts.get("liquid")) * 1e9 - 100) < 0.5,
  );
  const stirrer = bindInstrument(await load("heated-stirrer")),
    s = observation(stirrer);
  s.values.heat_dial_rad = Math.PI / 2;
  stirrer.apply(s);
  assert.ok(
    new Vector3(0, 1, 0).applyQuaternion(
      stirrer.parts.get("heat_dial").quaternion,
    ).x > 0.9,
  );
});

test("rotation spans the host observation window, freezes at its bound, and resets on actual state", async () => {
  const root = await load("heated-stirrer");
  const binding = bindInstrument(root, { predictionWindowSeconds: 5 });
  const state = observation(binding);
  Object.assign(state.values, { bar_rad: 0, bar_rad_s: 1 });
  binding.apply(state);
  const bar = binding.parts.get("bar");
  binding.animate(1);
  const afterOne = bar.quaternion.clone();
  binding.animate(1);
  assert.ok(!afterOne.equals(bar.quaternion));
  binding.animate(10);
  const last = bar.quaternion.clone();
  binding.animate(1);
  assert.ok(last.equals(bar.quaternion));
  binding.apply({ ...state, revision: 1 });
  binding.animate(1);
  assert.ok(afterOne.equals(bar.quaternion));
  assert.throws(
    () => bindInstrument(root, { predictionWindowSeconds: Infinity }),
    /prediction window/,
  );
  assert.throws(
    () => bindInstrument(root, { predictionWindowSeconds: -1 }),
    /prediction window/,
  );
});

test("orbital platform remains level throughout a 25mm orbit and carries the flask and clamps", async () => {
  const { Quaternion } = await import("three");
  const root = await load("orbital-shaker"),
    binding = bindInstrument(root),
    state = observation(binding);
  Object.assign(state.values, {
    orbit_rad: 0,
    platform_rad: 0,
    flask_visible: true,
    clamp_rad: 0,
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  const platform = binding.parts.get("platform"),
    eccentric = binding.parts.get("eccentric");
  const orientation = platform.getWorldQuaternion(new Quaternion());
  const first = platform.getWorldPosition(new Vector3());
  for (const angle of [Math.PI / 2, Math.PI, Math.PI * 1.5]) {
    Object.assign(state.values, { orbit_rad: angle, platform_rad: -angle });
    binding.apply(state);
    root.updateMatrixWorld(true);
    assert.ok(
      Math.abs(platform.getWorldQuaternion(new Quaternion()).dot(orientation)) >
        1 - 1e-8,
    );
    const offset = platform
      .getWorldPosition(new Vector3())
      .sub(eccentric.getWorldPosition(new Vector3()));
    assert.ok(Math.abs(Math.hypot(offset.x, offset.z) - 0.0125) < 1e-7);
    assert.ok(Math.abs(offset.y - 0.012) < 1e-7);
    for (const name of ["flask", "clamp_0", "clamp_1", "clamp_2"])
      assert.equal(binding.parts.get(name).parent, platform);
    if (angle === Math.PI)
      assert.ok(
        Math.abs(
          platform.getWorldPosition(new Vector3()).distanceTo(first) - 0.025,
        ) < 1e-7,
      );
  }
  const clamp = binding.parts.get("clamp_0"),
    closed = clamp.quaternion.clone();
  state.values.clamp_rad = 0.35;
  binding.apply(state);
  assert.ok(!clamp.quaternion.equals(closed));
  assert.deepEqual(binding.action(clamp.children[0], state.values), {
    action: "clamp",
    params: {},
  });
  const dial = binding.parts.get("speed_dial").children[0];
  assert.deepEqual(
    binding.action(dial, { ...state.values, target_rpm: 490 }, 1),
    { action: "set_speed", params: { rpm: 500 } },
  );
});

test("roller fixture has tangent support and overhead carriage clears the vessel rim before loading", async () => {
  const roller = bindInstrument(await load("tube-roller"));
  const rs = observation(roller);
  Object.assign(rs.values, { tube_visible: true, roller_rad: 0, tube_rad: 0 });
  roller.apply(rs);
  roller.parts.get("body").parent.updateMatrixWorld(true);
  const tubeCentre = roller.parts.get("tube").getWorldPosition(new Vector3());
  for (const name of ["roller_1", "roller_2"]) {
    const distance = tubeCentre.distanceTo(
      roller.parts.get(name).getWorldPosition(new Vector3()),
    );
    assert.ok(
      Math.abs(distance - 0.028) < 1e-7,
      "declared radii must meet without penetration",
    );
  }
  const overhead = bindInstrument(await load("overhead-stirrer"));
  const os = observation(overhead);
  Object.assign(os.values, {
    vessel_visible: true,
    carriage_m: 0.1,
    shaft_rad: 0,
    clamp_rad: 0,
  });
  overhead.apply(os);
  overhead.parts.get("body").parent.updateWorldMatrix(true, true);
  const shaft = new Box3().setFromObject(overhead.parts.get("shaft"));
  const vesselBounds = new Box3().setFromObject(overhead.parts.get("vessel"));
  assert.ok(
    shaft.min.y > vesselBounds.max.y + 0.04,
    "raised impeller must clear the rim",
  );
  os.values.carriage_m = 0;
  overhead.apply(os);
  overhead.parts.get("body").parent.updateWorldMatrix(true, true);
  shaft.setFromObject(overhead.parts.get("shaft"));
  assert.ok(
    shaft.min.y > vesselBounds.min.y + 0.003,
    "lowered impeller cannot strike the floor",
  );
  assert.ok(
    shaft.min.y < vesselBounds.min.y + 0.12,
    "paddle reaches the synthetic liquid",
  );
  for (const name of ["shaft", "clamp", "screen", "speed_dial"])
    assert.equal(
      overhead.parts.get(name).parent,
      overhead.parts.get("carriage"),
    );
});

test("dry block has real blind wells with floors and a supported cover that opens upward", async () => {
  const { Raycaster } = await import("three");
  const binding = bindInstrument(await load("dry-block-heater"));
  const state = observation(binding);
  Object.assign(state.values, { lid_rad: 1.7, sample_visible: false });
  binding.apply(state);
  binding.parts.get("body").parent.updateMatrixWorld(true);
  const ray = new Raycaster(new Vector3(0, 0.3, 0.007), new Vector3(0, -1, 0));
  let hits = ray.intersectObject(binding.parts.get("body"), true);
  assert.ok(hits.length > 0);
  assert.ok(
    Math.abs(hits[0].point.y - 0.085) < 0.0001,
    "well bottom leaves a 10mm floor",
  );
  ray.ray.origin.x = 0.012;
  hits = ray.intersectObject(binding.parts.get("body"), true);
  assert.ok(
    Math.abs(hits[0].point.y - 0.125) < 0.0001,
    "solid block surrounds the bore",
  );
  const lid = new Box3().setFromObject(binding.parts.get("lid"));
  assert.ok(
    lid.max.y > 0.25,
    "cover sweeps above the tubes rather than through the block",
  );
});

test("peristaltic tube has an open lumen, the declared 7mL fluid volume and rolling followers", async () => {
  const { Quaternion, Raycaster } = await import("three");
  const root = await load("peristaltic-pump"),
    binding = bindInstrument(root),
    state = observation(binding);
  Object.assign(state.values, {
    rotor_rad: 0,
    roller_rad: 0,
    liquid_visible: true,
    tube_visible: true,
    source_fraction: 1,
    receiver_fraction: 0,
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  const volume = meshVolume(binding.parts.get("liquid")) * 1e6;
  assert.ok(Math.abs(volume - 7) < 0.14, `actual lumen volume ${volume}mL`);
  const ray = new Raycaster(
    new Vector3(-0.17, 0.028, 0.035),
    new Vector3(0, 1, 0),
    0,
    0.004,
  );
  assert.equal(
    ray.intersectObject(binding.parts.get("tubing"), true).length,
    0,
    "tube inlet must have an open bore",
  );
  const follower = binding.parts.get("roller_0"),
    rest = follower.getWorldQuaternion(new Quaternion());
  const angle = 0.3;
  Object.assign(state.values, {
    rotor_rad: angle,
    roller_rad: (-4 * angle + Math.PI * 2) % (Math.PI * 2),
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  const delta = follower
    .getWorldQuaternion(new Quaternion())
    .multiply(rest.invert());
  assert.ok(
    Math.abs(
      delta.dot(
        new Quaternion().setFromAxisAngle(new Vector3(0, 0, -1), -3 * angle),
      ),
    ) >
      1 - 1e-8,
  );
});

test("ultrasonic tank water volume matches thermal parameters and the basket rests on its floor", async () => {
  const root = await load("ultrasonic-cleaner"),
    binding = bindInstrument(root),
    state = observation(binding);
  Object.assign(state.values, {
    lid_rad: 1.8,
    fill_fraction: 1,
    water_visible: true,
    sample_visible: true,
  });
  binding.apply(state);
  root.updateMatrixWorld(true);
  const spec = JSON.parse(
    await readFile(new URL("ultrasonic-cleaner/model.yaml", folder)),
  );
  const water = binding.parts.get("water"),
    full = meshVolume(water) * 1e6;
  assert.ok(
    Math.abs(full - spec.runtime.water_capacity_j_k / 4.18) < full * 0.01,
  );
  const basket = new Box3().setFromObject(binding.parts.get("sample"));
  assert.ok(
    Math.abs(basket.min.y - 0.082) < 1e-6,
    "basket legs must rest on the basin floor",
  );
  assert.ok(
    basket.min.x > -0.128 &&
      basket.max.x < 0.128 &&
      basket.min.z > -0.09 &&
      basket.max.z < 0.09,
  );
  state.values.fill_fraction = 0.5;
  binding.apply(state);
  root.updateMatrixWorld(true);
  const half = new Box3().setFromObject(water);
  assert.ok(Math.abs(half.min.y - 0.082) < 1e-6);
  assert.ok(Math.abs(half.getSize(new Vector3()).y - 0.0375) < 1e-6);
});
