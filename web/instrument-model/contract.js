export const finite = (v) => typeof v === "number" && Number.isFinite(v);
const record = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
export const id = (v) =>
  typeof v === "string" && /^[a-zA-Z0-9_.-]{1,128}$/.test(v);
export function requireValue(ok, message) {
  if (!ok) throw new Error(`Invalid instrument model: ${message}`);
}
function closed(value, keys) {
  return record(value) && Object.keys(value).every((key) => keys.includes(key));
}
function vector(value) {
  return Array.isArray(value) && value.length === 3 && value.every(finite);
}

/** Parse once at asset admission/loading; no interpretation of names or executable code. */
export function parseModel(value) {
  requireValue(
    closed(value, [
      "schema_version",
      "model_id",
      "dimensions_m",
      "components",
      "joints",
      "readouts",
      "visibility",
      "placements",
    ]) &&
      value.schema_version === 1 &&
      id(value.model_id),
    "identity/version",
  );
  requireValue(
    vector(value.dimensions_m) &&
      value.dimensions_m.every((n) => n > 0 && n < 100),
    "dimensions",
  );
  for (const key of [
    "components",
    "joints",
    "readouts",
    "visibility",
    "placements",
  ])
    requireValue(Array.isArray(value[key]) && value[key].length <= 256, key);
  const components = new Map([["instrument", null]]);
  for (const c of value.components) {
    requireValue(
      closed(c, ["id", "label", "parent", "control", "scale"]) &&
        id(c.id) &&
        !components.has(c.id) &&
        id(c.parent) &&
        typeof c.label === "string" &&
        c.label.length <= 512,
      "component",
    );
    components.set(c.id, c);
    if (c.scale !== undefined && c.scale !== null) {
      const s = c.scale;
      requireValue(
        closed(s, ["axis", "state", "min", "max"]) &&
          ["x", "y", "z"].includes(s.axis) &&
          id(s.state) &&
          finite(s.min) &&
          finite(s.max) &&
          s.min >= 0 &&
          s.min < s.max,
        "component scale",
      );
    }
    if (c.control !== undefined && c.control !== null) {
      const t = c.control;
      requireValue(
        closed(
          t,
          t.kind === "command"
            ? ["kind", "action", "params"]
            : t.kind === "dial"
              ? [
                  "kind",
                  "action",
                  "params",
                  "parameter",
                  "state",
                  "min",
                  "max",
                  "step",
                ]
              : [
                  "kind",
                  "action",
                  "params",
                  "parameter",
                  "state",
                  "open",
                  "closed",
                ],
        ) &&
          ["command", "toggle", "dial"].includes(t.kind) &&
          id(t.action) &&
          record(t.params) &&
          Object.keys(t.params).length <= 16 &&
          Object.entries(t.params).every(
            ([k, v]) =>
              id(k) &&
              (finite(v) ||
                typeof v === "boolean" ||
                (typeof v === "string" && v.length < 512)),
          ),
        "control",
      );
      if (t.kind === "dial")
        requireValue(
          id(t.parameter) &&
            id(t.state) &&
            finite(t.min) &&
            finite(t.max) &&
            t.min < t.max &&
            finite(t.step) &&
            t.step > 0 &&
            t.step <= t.max - t.min,
          "dial",
        );
      if (t.kind === "toggle")
        requireValue(
          id(t.parameter) && id(t.state) && finite(t.open) && finite(t.closed),
          "toggle",
        );
    }
  }
  for (const c of value.components) {
    const seen = new Set([c.id]);
    for (
      let parent = c.parent;
      parent !== "instrument";
      parent = components.get(parent)?.parent
    ) {
      requireValue(
        components.has(parent) && !seen.has(parent),
        "component cycle/parent",
      );
      seen.add(parent);
    }
  }
  const jointIds = new Set(),
    moving = new Set();
  for (const j of value.joints) {
    requireValue(
      closed(j, [
        "id",
        "component",
        "kind",
        "axis",
        "min",
        "max",
        "state",
        "velocity_state",
      ]) &&
        id(j.id) &&
        !jointIds.has(j.id) &&
        j.component !== "instrument" &&
        components.has(j.component) &&
        !moving.has(j.component),
      "joint identity",
    );
    requireValue(
      ["prismatic", "revolute", "continuous"].includes(j.kind) &&
        vector(j.axis) &&
        Math.abs(Math.hypot(...j.axis) - 1) < 1e-6 &&
        finite(j.min) &&
        finite(j.max) &&
        j.min <= j.max &&
        id(j.state),
      "joint frame/limits",
    );
    requireValue(
      j.kind === "continuous"
        ? id(j.velocity_state)
        : j.velocity_state === undefined || j.velocity_state === null,
      "joint velocity",
    );
    jointIds.add(j.id);
    moving.add(j.component);
  }
  for (const b of [...value.readouts, ...value.visibility])
    requireValue(
      record(b) &&
        b.component !== "instrument" &&
        components.has(b.component) &&
        id(b.state),
      "binding",
    );
  for (const b of value.visibility)
    requireValue(closed(b, ["component", "state"]), "visibility fields");
  for (const r of value.readouts)
    requireValue(
      closed(r, ["component", "state", "digits", "unit"]) &&
        Number.isInteger(r.digits) &&
        r.digits >= 0 &&
        r.digits <= 8 &&
        typeof r.unit === "string" &&
        r.unit.length <= 32,
      "readout",
    );
  for (const p of value.placements)
    requireValue(
      closed(p, ["id", "component", "position", "radius_m"]) &&
        id(p.id) &&
        p.component !== "instrument" &&
        components.has(p.component) &&
        vector(p.position) &&
        finite(p.radius_m) &&
        p.radius_m > 0,
      "placement",
    );
  return value;
}

export function parseModelState(value) {
  if (
    !closed(value, [
      "schema_version",
      "model_id",
      "run_id",
      "revision",
      "time_s",
      "values",
      "motion",
    ]) ||
    value.schema_version !== 2 ||
    !id(value.model_id) ||
    !id(value.run_id) ||
    !Number.isSafeInteger(value.revision) ||
    value.revision < 0 ||
    !finite(value.time_s) ||
    value.time_s < 0 ||
    !record(value.values) ||
    Object.keys(value.values).length > 256 ||
    !record(value.motion) ||
    Object.keys(value.motion).length > 256
  )
    return null;
  if (
    !Object.entries(value.values).every(
      ([k, v]) =>
        id(k) &&
        (v === null ||
          finite(v) ||
          typeof v === "boolean" ||
          (typeof v === "string" && v.length < 512)),
    )
  )
    return null;
  for (const [key, motion] of Object.entries(value.motion))
    if (
      !id(key) ||
      !finite(value.values[key]) ||
      !closed(motion, ["target_velocity", "slew_per_s", "stop_after_s"]) ||
      !finite(motion.target_velocity) ||
      !finite(motion.slew_per_s) ||
      motion.slew_per_s <= 0 ||
      !(
        motion.stop_after_s === null ||
        (finite(motion.stop_after_s) && motion.stop_after_s >= 0)
      )
    )
      return null;
  return value;
}

export function formatReadout(readout, values) {
  const value = values[readout.state];
  return typeof value === "number" && Number.isFinite(value)
    ? `${value.toFixed(readout.digits)} ${readout.unit}`
    : "—";
}
