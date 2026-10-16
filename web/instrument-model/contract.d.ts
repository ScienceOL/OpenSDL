export type ModelValues = Record<string, number | boolean | string | null>;
export type ModelControl =
  | {
      kind: "command";
      action: string;
      params: Record<string, string | number | boolean>;
    }
  | {
      kind: "toggle";
      action: string;
      params: Record<string, string | number | boolean>;
      parameter: string;
      state: string;
      open: number;
      closed: number;
    }
  | {
      kind: "dial";
      action: string;
      params: Record<string, string | number | boolean>;
      parameter: string;
      state: string;
      min: number;
      max: number;
      step: number;
    };
export interface ModelDefinition {
  schema_version: 1;
  model_id: string;
  dimensions_m: [number, number, number];
  components: {
    id: string;
    label: string;
    parent: string;
    control?: ModelControl | null;
    scale?: {
      axis: "x" | "y" | "z";
      state: string;
      min: number;
      max: number;
    } | null;
  }[];
  joints: {
    id: string;
    component: string;
    kind: "prismatic" | "revolute" | "continuous";
    axis: [number, number, number];
    min: number;
    max: number;
    state: string;
    velocity_state?: string | null;
  }[];
  readouts: {
    component: string;
    state: string;
    digits: number;
    unit: string;
  }[];
  visibility: { component: string; state: string }[];
  placements: {
    id: string;
    component: string;
    position: [number, number, number];
    radius_m: number;
  }[];
}
export interface ModelState {
  schema_version: 1;
  run_id: string;
  model_id: string;
  revision: number;
  time_s: number;
  values: ModelValues;
}
export interface ModelAction {
  action: string;
  params: Record<string, string | number | boolean>;
}

export function parseModel(value: unknown): ModelDefinition;
export function parseModelState(value: unknown): ModelState | null;
export function formatReadout(
  readout: ModelDefinition["readouts"][number],
  values: ModelValues,
): string;
