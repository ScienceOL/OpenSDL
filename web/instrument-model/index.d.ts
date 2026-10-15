import type {
  ModelDefinition,
  ModelState,
  ModelAction,
  ModelValues,
} from "./contract.js";
export * from "./contract.js";
/** Structural scene boundary; hosts retain ownership of their Three.js types and resources. */
export interface InstrumentNode {
  parent: InstrumentNode | null;
  userData: Record<string, unknown>;
  traverse(callback: (node: InstrumentNode) => void): void;
}
export interface InstrumentBinding {
  definition: ModelDefinition;
  apply(state: ModelState): void;
  inspect(jointId: string, value: number): void;
  action(object: InstrumentNode, values: ModelValues): ModelAction | null;
}
export function bindInstrument(root: InstrumentNode): InstrumentBinding | null;
export function createReadouts(binding: InstrumentBinding): {
  update(values: ModelValues): void;
  dispose(): void;
};
