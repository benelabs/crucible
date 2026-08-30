/// <reference lib="webworker" />
/**
 * Worker body for {@link BrowserWasmRunner}.
 *
 * Deliberately thin: it owns no policy, only the isolation. All metering
 * lives in `executeMetered`, so the same code path is what tests exercise
 * directly and what runs here.
 */
import { executeMetered, type SandboxRequest } from './BrowserWasmRunner';

self.onmessage = async (event: MessageEvent<SandboxRequest>) => {
  const { id, wasm, fn, args, gasLimit, costs } = event.data;
  const result = await executeMetered(wasm, fn, args, { gasLimit, costs });
  (self as unknown as Worker).postMessage({ id, result });
};
