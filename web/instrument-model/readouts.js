import { formatReadout } from "./contract.js";
import { CanvasTexture, Mesh, MeshBasicMaterial, SRGBColorSpace } from "three";

/** The host releases these instance-owned surfaces before releasing shared asset resources. */
export function createReadouts(binding) {
  const surfaces = binding.definition.readouts.map((readout) => {
    const canvas = document.createElement("canvas");
    canvas.width = 768;
    canvas.height = 192;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Readout canvas unavailable");
    const texture = new CanvasTexture(canvas);
    texture.colorSpace = SRGBColorSpace;
    texture.flipY = false;
    const material = new MeshBasicMaterial({ map: texture, toneMapped: false });
    const meshes = [];
    binding.parts.get(readout.component).traverse((node) => {
      if (node instanceof Mesh) {
        meshes.push([node, node.material]);
        node.material = material;
      }
    });
    return { readout, context, texture, material, meshes, last: undefined };
  });
  return {
    update(values) {
      for (const surface of surfaces) {
        const { readout, context, texture } = surface;
        const text = formatReadout(readout, values);
        if (text === surface.last) continue;
        surface.last = text;
        context.fillStyle = "#101e23";
        context.fillRect(0, 0, 768, 192);
        context.fillStyle = "#beeee2";
        context.font = "96px monospace";
        context.textAlign = "right";
        context.textBaseline = "middle";
        context.fillText(text, 736, 98, 710);
        texture.needsUpdate = true;
      }
    },
    dispose() {
      for (const surface of surfaces) {
        for (const [mesh, original] of surface.meshes) mesh.material = original;
        surface.texture.dispose();
        surface.material.dispose();
      }
    },
  };
}
