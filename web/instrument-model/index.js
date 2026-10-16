import { Quaternion, Vector3 } from "three";

import {
  parseModel,
  parseModelState,
  requireValue,
  finite,
  id,
} from "./contract.js";
export { parseModel, parseModelState, formatReadout } from "./contract.js";

/** A binding owns only instance transforms. Geometry/material lifetimes belong to the host. */
export function bindInstrument(root) {
  let declaration;
  const parts = new Map();
  root.traverse((node) => {
    if (node.userData.opensdlModel !== undefined) {
      requireValue(!declaration, "multiple model roots");
      declaration = node.userData.opensdlModel;
    }
    const key = node.userData.opensdlComponent;
    if (key !== undefined) {
      requireValue(id(key) && !parts.has(key), "duplicate component node");
      parts.set(key, node);
    }
  });
  if (declaration === undefined) return null;
  const definition = parseModel(declaration);
  for (const c of definition.components) {
    const node = parts.get(c.id),
      parent = parts.get(c.parent);
    requireValue(
      node && parent && node.parent === parent,
      `node parent ${c.id}`,
    );
  }
  const controls = new Map(
    definition.components
      .filter((c) => c.control)
      .map((c) => [parts.get(c.id), c.control]),
  );
  for (const binding of definition.visibility)
    parts.get(binding.component).visible = false;
  const rest = new Map(
    definition.joints.map((j) => {
      const node = parts.get(j.component);
      return [
        j.id,
        {
          position: node.position.clone(),
          quaternion: node.quaternion.clone(),
          rotation: new Quaternion(),
          axis: new Vector3(...j.axis),
        },
      ];
    }),
  );
  const scaling = definition.components
    .filter((c) => c.scale)
    .map((c) => ({
      node: parts.get(c.id),
      binding: c.scale,
      rest: parts.get(c.id).scale.clone(),
    }));
  function pose(joint, value) {
    requireValue(
      finite(value) &&
        (joint.kind === "continuous" ||
          (value >= joint.min && value <= joint.max)),
      `pose ${joint.id}`,
    );
    const node = parts.get(joint.component),
      initial = rest.get(joint.id);
    if (joint.kind === "prismatic")
      node.position.copy(initial.position).addScaledVector(initial.axis, value);
    else
      node.quaternion
        .copy(initial.quaternion)
        .multiply(initial.rotation.setFromAxisAngle(initial.axis, value));
  }
  let elapsed = 0,
    observation = null;
  return {
    definition,
    parts,
    apply(state) {
      requireValue(
        parseModelState(state) && state.model_id === definition.model_id,
        "state belongs to a different model",
      );
      // Validate the complete affected pose before mutating any node.
      for (const j of definition.joints)
        requireValue(
          finite(state.values[j.state]) &&
            (j.kind === "continuous" ||
              (state.values[j.state] >= j.min &&
                state.values[j.state] <= j.max)),
          `state ${j.state}`,
        );
      for (const b of definition.visibility)
        requireValue(
          typeof state.values[b.state] === "boolean",
          `visibility ${b.state}`,
        );
      for (const r of definition.readouts)
        requireValue(
          state.values[r.state] === null || finite(state.values[r.state]),
          `readout ${r.state}`,
        );
      for (const j of definition.joints)
        if (j.kind === "continuous")
          requireValue(finite(state.values[j.velocity_state]), "joint speed");
      for (const s of scaling)
        requireValue(
          finite(state.values[s.binding.state]) &&
            state.values[s.binding.state] >= s.binding.min &&
            state.values[s.binding.state] <= s.binding.max,
          "component scale state",
        );
      observation = state;
      elapsed = 0;
      for (const s of scaling) {
        s.node.scale.copy(s.rest);
        s.node.scale[s.binding.axis] =
          s.rest[s.binding.axis] * state.values[s.binding.state];
      }
      for (const j of definition.joints) pose(j, state.values[j.state]);
      for (const b of definition.visibility)
        parts.get(b.component).visible = state.values[b.state];
    },
    animate(dt) {
      if (!observation || !finite(dt) || dt <= 0) return;
      // Predict at most 250 ms from an actual observation. Offline/stalled transport cannot spin forever.
      const next = Math.min(0.25, elapsed + dt);
      if (next === elapsed) return;
      elapsed = next;
      for (const j of definition.joints)
        if (j.kind === "continuous")
          pose(
            j,
            observation.values[j.state] +
              observation.values[j.velocity_state] * elapsed,
          );
    },
    inspect(jointId, value) {
      const j = definition.joints.find((j) => j.id === jointId);
      requireValue(j, "unknown joint");
      pose(j, value);
    },
    control(object) {
      let node = object;
      while (node) {
        const control = controls.get(node);
        if (control) return control;
        if (node === root) break;
        node = node.parent;
      }
      return null;
    },
    action(object, values, steps = 1) {
      const t = this.control(object);
      if (!t) return null;
      const params = { ...t.params };
      if (t.kind === "toggle") {
        requireValue(finite(values[t.state]), "unconfirmed toggle state");
        params[t.parameter] =
          values[t.state] > (t.open + t.closed) / 2 ? t.closed : t.open;
      }
      if (t.kind === "dial") {
        requireValue(
          finite(values[t.state]) && finite(steps),
          "unconfirmed dial state",
        );
        params[t.parameter] = Math.min(
          t.max,
          Math.max(t.min, values[t.state] + steps * t.step),
        );
      }
      return { action: t.action, params };
    },
  };
}

export { createReadouts } from "./readouts.js";
