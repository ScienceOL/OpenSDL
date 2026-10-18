import type {
  ModelDefinition,
  ModelState,
  ModelAction,
  ModelValues,
  ModelControl,
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
  control(object: InstrumentNode): ModelControl | null;
  apply(state: ModelState): void;
  animate(dt: number): void;
  inspect(jointId: string, value: number): void;
  action(
    object: InstrumentNode,
    values: ModelValues,
    steps?: number,
  ): ModelAction | null;
}
export function bindInstrument(
  root: InstrumentNode,
  options?: {
    /** Maximum forward rotation from an actual observation. Default: 0.25 s. */
    predictionWindowSeconds?: number;
  },
): InstrumentBinding | null;
export function createReadouts(binding: InstrumentBinding): {
  update(values: ModelValues): void;
  dispose(): void;
};
