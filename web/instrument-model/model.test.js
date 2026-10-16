import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { Box3, Mesh, Object3D, Vector3 } from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { bindInstrument, parseModel, parseModelState } from "./index.js";
const asset = new URL(
  "../../assets/instruments/analytical-balance/package/preview.glb",
  import.meta.url,
);
async function load() {
  const bytes = await readFile(asset);
  return (
    await new GLTFLoader().parseAsync(
      bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength),
      "",
    )
  ).scene;
}
function state(values = {}) {
  return {
    schema_version: 1,
    run_id: "test-run",
    model_id: "opensdl.ax240",
    revision: 1,
    time_s: 0,
    values: {
      door_left_m: 0,
      door_right_m: 0,
      door_top_m: 0,
      mass_g: null,
      vessel_visible: false,
      mass_5g_visible: false,
      mass_20g_visible: false,
      overload_visible: false,
      ...values,
    },
  };
}
test("actual exported model retains SI scale, independent parts, and complete door travel", async () => {
  const root = await load(),
    binding = bindInstrument(root);
  assert.ok(binding);
  binding.apply(state());
  const bounds = new Box3().setFromObject(root),
    size = bounds.getSize(new Vector3());
  const expected = [0.236, 0.336, 0.354];
  for (let i = 0; i < 3; i++)
    assert.ok(
      Math.abs(size.getComponent(i) - expected[i]) < 0.001,
      `axis ${i}: ${size.getComponent(i)}`,
    );
  assert.deepEqual(binding.definition.dimensions_m, expected);
  const left = binding.parts.get("door_left"),
    right = binding.parts.get("door_right");
  binding.apply(state({ door_left_m: 0.06 }));
  assert.equal(left.position.z, -0.06);
  assert.equal(right.position.z, 0);
  binding.apply(
    state({ door_left_m: 0.12, vessel_visible: true, mass_g: 18.4 }),
  );
  assert.equal(left.position.z, -0.12);
  assert.equal(binding.parts.get("vessel").visible, true);
  assert.deepEqual(binding.action(left.children[0], { door_left_m: 0.12 }), {
    action: "set_door",
    params: { door: "left", opening_m: 0 },
  });
  assert.deepEqual(
    binding.action(binding.parts.get("tare_key").children[0], {}),
    { action: "tare", params: {} },
  );
  assert.throws(() => binding.apply(state({ door_left_m: 0.121 })), /state/);
  assert.equal(
    left.position.z,
    -0.12,
    "invalid pose must not partly change geometry",
  );
  const second = bindInstrument(root.clone(true));
  second.apply(state());
  assert.equal(left.position.z, -0.12);
  assert.equal(
    second.parts.get("door_left").position.z,
    -0.12,
    "clone rest pose captured at binding time",
  );
});
test("prototype clones preserve authored rest transforms and share only immutable geometry", async () => {
  const prototype = await load(),
    a = prototype.clone(true),
    b = prototype.clone(true);
  const first = bindInstrument(a),
    second = bindInstrument(b);
  first.apply(state({ door_top_m: 0.12 }));
  second.apply(state({ door_top_m: 0 }));
  assert.equal(first.parts.get("door_top").position.z, -0.12);
  assert.equal(second.parts.get("door_top").position.z, 0);
  let triangles = 0;
  prototype.traverse((n) => {
    if (n instanceof Mesh)
      triangles +=
        (n.geometry.index?.count ?? n.geometry.attributes.position.count) / 3;
  });
  assert.ok(triangles < 30000);
  assert.ok((await readFile(asset)).length < 4 * 1024 * 1024);
});
test("strict portable structure rejects executable or ambiguous bindings", async () => {
  const definition = bindInstrument(await load()).definition;
  for (const mutate of [
    (d) => (d.extra = true),
    (d) => (d.components[0].parent = d.components[0].id),
    (d) => (d.joints[0].axis = [2, 0, 0]),
    (d) => (d.joints[0].component = "instrument"),
    (d) =>
      (d.components[0].control = {
        kind: "command",
        action: "run",
        params: { script: { code: "x" } },
      }),
  ]) {
    const value = structuredClone(definition);
    mutate(value);
    assert.throws(() => parseModel(value));
  }
  assert.equal(parseModelState({ ...state(), revision: Infinity }), null);
  assert.equal(
    parseModelState({ ...state(), values: { script: { code: "x" } } }),
    null,
  );
});

test("authored sources and generated package have matching provenance", async () => {
  const { createHash } = await import("node:crypto");
  const provenance = JSON.parse(
    await readFile(
      new URL(
        "../../assets/instruments/analytical-balance/package/provenance.json",
        import.meta.url,
      ),
    ),
  );
  for (const [path, expected] of Object.entries(provenance.inputs)) {
    const bytes = await readFile(new URL(`../../${path}`, import.meta.url));
    assert.equal(
      createHash("sha256").update(bytes).digest("hex"),
      expected,
      path,
    );
  }
  const usd = await readFile(
    new URL(
      "../../assets/instruments/analytical-balance/package/source/model.usda",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(usd, /metersPerUnit = 1/);
  assert.match(usd, /upAxis = "Z"/);
});

test("fixture geometry rests on the pan at the authored footprint and height", async () => {
  const root = await load(),
    binding = bindInstrument(root);
  const design = JSON.parse(
    await readFile(
      new URL(
        "../../assets/instruments/analytical-balance/model.yaml",
        import.meta.url,
      ),
    ),
  );
  root.updateMatrixWorld(true);
  for (const [id, fixture] of Object.entries(design.fixtures)) {
    const box = new Box3().setFromObject(binding.parts.get(id)),
      size = box.getSize(new Vector3());
    assert.ok(
      Math.abs(box.min.y - design.pan_height_m) < 1e-6,
      `${id} is unsupported`,
    );
    assert.ok(
      Math.abs(size.x - 2 * fixture.radius_m) < 1e-6,
      `${id} footprint`,
    );
    const height =
      fixture.height_m ??
      fixture.mass_kg /
        (fixture.density_kg_m3 * Math.PI * fixture.radius_m ** 2);
    assert.ok(Math.abs(size.y - height) < 1e-6, `${id} height`);
  }
});

test("readout surfaces are instance-owned and redraw only a changed value", async () => {
  const { createReadouts } = await import("./index.js");
  const previous = globalThis.document,
    draws = [];
  globalThis.document = {
    createElement: () => ({
      width: 0,
      height: 0,
      getContext: () => ({
        fillRect() {},
        fillText(text) {
          draws.push(text);
        },
      }),
    }),
  };
  try {
    const prototype = await load(),
      a = bindInstrument(prototype.clone(true)),
      b = bindInstrument(prototype.clone(true));
    const mesh = a.parts.get("screen").children.find((node) => node.isMesh);
    const other = b.parts.get("screen").children.find((node) => node.isMesh);
    const original = mesh.material,
      readout = createReadouts(a);
    assert.notEqual(mesh.material, original);
    assert.equal(other.material, original);
    let disposed = 0;
    mesh.material.addEventListener("dispose", () => disposed++);
    readout.update({ mass_g: 18.4 });
    readout.update({ mass_g: 18.4 });
    assert.deepEqual(draws, ["18.4000 g"]);
    readout.dispose();
    assert.equal(disposed, 1);
    assert.equal(mesh.material, original);
    assert.equal(other.material, original);
  } finally {
    globalThis.document = previous;
  }
});

// Rigid consolidation joins disconnected solids. Recover their connected vertex
// groups so an empty gap between two rails is not treated as occupied geometry.
function solidBounds(mesh) {
  const positions = mesh.geometry.attributes.position;
  const points = [],
    ids = [],
    parents = [],
    byPosition = new Map();
  const find = (n) => {
    while (parents[n] !== n) {
      parents[n] = parents[parents[n]];
      n = parents[n];
    }
    return n;
  };
  for (let i = 0; i < positions.count; i++) {
    const point = new Vector3().fromBufferAttribute(positions, i);
    const key = point
      .toArray()
      .map((n) => n.toFixed(6))
      .join(",");
    let id = byPosition.get(key);
    if (id === undefined) {
      id = parents.length;
      parents.push(id);
      byPosition.set(key, id);
    }
    ids.push(id);
    points.push(point.applyMatrix4(mesh.matrixWorld));
  }
  const indices =
    mesh.geometry.index?.array ??
    Array.from({ length: positions.count }, (_, i) => i);
  for (let i = 0; i < indices.length; i += 3) {
    const root = find(ids[indices[i]]);
    parents[find(ids[indices[i + 1]])] = root;
    parents[find(ids[indices[i + 2]])] = root;
  }
  const bounds = new Map();
  for (let i = 0; i < points.length; i++) {
    const id = find(ids[i]);
    if (!bounds.has(id)) bounds.set(id, new Box3());
    bounds.get(id).expandByPoint(points[i]);
  }
  return [...bounds.values()];
}
test("every draft-shield door has a clear swept path through its complete travel", async () => {
  const root = await load(),
    binding = bindInstrument(root);
  root.updateMatrixWorld(true);
  const fixed = [];
  binding.parts.get("draft_shield").traverse((node) => {
    if (node.isMesh) fixed.push(...solidBounds(node));
  });
  for (const joint of binding.definition.joints) {
    const travel = new Vector3(...joint.axis).multiplyScalar(joint.max);
    binding.parts.get(joint.component).traverse((node) => {
      if (!node.isMesh) return;
      for (const part of solidBounds(node)) {
        const swept = part.clone().union(part.clone().translate(travel));
        for (const obstacle of fixed) {
          const intersection = swept.clone().intersect(obstacle);
          if (intersection.isEmpty()) continue;
          const size = intersection.getSize(new Vector3());
          assert.ok(
            Math.min(size.x, size.y, size.z) <= 1e-6,
            `${joint.id}: ${node.name} intersects a fixed solid`,
          );
        }
      }
    });
  }
});

test("the hollow vessel has a closed centre floor, rather than a toroidal hole", async () => {
  const { Raycaster } = await import("three");
  const root = await load(),
    binding = bindInstrument(root);
  binding.apply(state({ vessel_visible: true }));
  root.updateMatrixWorld(true);
  const hits = new Raycaster(
    new Vector3(0, 1, -0.041),
    new Vector3(0, -1, 0),
  ).intersectObject(binding.parts.get("vessel"), true);
  assert.ok(hits.some((hit) => Math.abs(hit.point.y - 0.0975) < 1e-6));
});

test("controls resolve only validated node identities, even if extras are modified", async () => {
  const binding = bindInstrument(await load());
  const left = binding.parts.get("door_left"),
    mesh = left.children[0];
  mesh.userData.opensdlComponent = "tare_key";
  left.userData.opensdlComponent = "zero_key";
  assert.equal(binding.control(mesh).action, "set_door");
  assert.deepEqual(binding.action(mesh, { door_left_m: 0 }), {
    action: "set_door",
    params: { door: "left", opening_m: 0.12 },
  });
  const foreign = new Object3D();
  foreign.userData.opensdlComponent = "tare_key";
  assert.equal(binding.control(foreign), null);
});
