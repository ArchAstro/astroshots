import { parentPort } from "node:worker_threads";

import { scaleImage, type ScaledFormat } from "./scale.js";

export interface WorkerRequest {
  id: number;
  bytes: ArrayBuffer;
  targetWidth: number;
  targetHeight: number;
  format: ScaledFormat;
}

export type WorkerResponse =
  | { id: number; ok: true; width: number; height: number; format: ScaledFormat; data: ArrayBuffer }
  | { id: number; ok: false; error: string };

parentPort?.on("message", (request: WorkerRequest) => {
  try {
    const scaled = scaleImage({
      bytes: Buffer.from(request.bytes),
      target: { width: request.targetWidth, height: request.targetHeight },
      format: request.format,
    });
    const data = scaled.data.buffer.slice(
      scaled.data.byteOffset,
      scaled.data.byteOffset + scaled.data.byteLength,
    ) as ArrayBuffer;
    const response: WorkerResponse = {
      id: request.id,
      ok: true,
      width: scaled.width,
      height: scaled.height,
      format: scaled.format,
      data,
    };
    parentPort?.postMessage(response, [data]);
  } catch (error) {
    const response: WorkerResponse = {
      id: request.id,
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    };
    parentPort?.postMessage(response);
  }
});
