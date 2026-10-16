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
  function volume(part) {
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
  assert.ok(Math.abs(volume(binding.parts.get("liquid")) * 1e9 - 100) < 0.5);
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
