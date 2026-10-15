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
          axis: new Vector3(...j.axis),
        },
      ];
    }),
  );
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
        .multiply(new Quaternion().setFromAxisAngle(initial.axis, value));
  }
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
      for (const j of definition.joints) pose(j, state.values[j.state]);
      for (const b of definition.visibility)
        parts.get(b.component).visible = state.values[b.state];
    },
    inspect(jointId, value) {
      const j = definition.joints.find((j) => j.id === jointId);
      requireValue(j, "unknown joint");
      pose(j, value);
    },
    action(object, values) {
      let node = object;
      while (node && node !== root.parent) {
        const c = definition.components.find(
          (c) => c.id === node.userData.opensdlComponent,
        );
        if (c?.control) {
          const t = c.control,
            params = { ...t.params };
          if (t.kind === "toggle") {
            requireValue(finite(values[t.state]), "unconfirmed toggle state");
            params[t.parameter] =
              values[t.state] > (t.open + t.closed) / 2 ? t.closed : t.open;
          }
          return { action: t.action, params };
        }
        node = node.parent;
      }
      return null;
    },
  };
}

export { createReadouts } from "./readouts.js";
