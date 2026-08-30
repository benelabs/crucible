import { describe, expect, it, vi } from 'vitest';

import {
  BrowserWasmRunner,
  DEFAULT_GAS_COSTS,
  executeMetered,
  type SandboxRequest,
  type SandboxResponse,
  type SandboxWorkerLike,
} from './BrowserWasmRunner';

/**
 * A hand-assembled Wasm module exporting:
 *  - `add(i32, i32) -> i32`      — pure computation, no host calls
 *  - `log_sum(i32, i32) -> i32`  — calls the imported `env.host_log`
 *  - `spin()`                    — a ~50M iteration loop, for the timeout path
 */
const FIXTURE_WASM_B64 =
  'AGFzbQEAAAABDgNgAn9/AX9gAX8AYAAAAhABA2Vudghob3N0X2xvZwABAwQDAAACBxgDA2FkZAABB2xvZ19zdW0AAgRzcGluAAMKMwMHACAAIAFqCxEBAX8gACABaiECIAIQACACCxcBAX9BgOHrFyEAA0AgAEEBayIADQALCw==';

function fixtureBytes(): Uint8Array<ArrayBuffer> {
  const binary = atob(FIXTURE_WASM_B64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

describe('executeMetered', () => {
  it('runs a pure export and returns its value', async () => {
    const result = await executeMetered(fixtureBytes(), 'add', [2, 3]);

    expect(result.success).toBe(true);
    expect(result.returnValue).toBe('5');
    expect(result.hostCalls).toBe(0);
  });

  it('charges no gas for computation that never enters the host', async () => {
    // Gas here meters host interaction, not raw instruction count — matching
    // how the backend runner prices a call.
    const result = await executeMetered(fixtureBytes(), 'add', [1, 1]);
    expect(result.gasConsumed).toBe(0);
  });

  it('charges gas per host call and records the trace', async () => {
    const result = await executeMetered(fixtureBytes(), 'log_sum', [4, 5]);

    expect(result.success).toBe(true);
    expect(result.returnValue).toBe('9');
    expect(result.hostCalls).toBe(1);
    expect(result.gasConsumed).toBe(
      DEFAULT_GAS_COSTS.hostCallBase + DEFAULT_GAS_COSTS.byteCost * 4,
    );
    expect(result.traceLogs).toContain('log: 9');
  });

  it('aborts the run when the gas limit is exceeded', async () => {
    const result = await executeMetered(fixtureBytes(), 'log_sum', [1, 1], {
      gasLimit: 10,
    });

    expect(result.success).toBe(false);
    expect(result.errorCode).toBe('gas_exhausted');
    expect(result.returnValue).toBe('');
  });

  it('reports a missing export instead of throwing', async () => {
    const result = await executeMetered(fixtureBytes(), 'nope', []);

    expect(result.success).toBe(false);
    expect(result.errorCode).toBe('export_not_found');
    expect(result.errorMessage).toContain('nope');
  });

  it('reports an invalid module instead of throwing', async () => {
    const result = await executeMetered(new Uint8Array([1, 2, 3, 4]), 'add', []);

    expect(result.success).toBe(false);
    expect(result.errorCode).toBe('invalid_module');
  });

  it('fails loudly on a host function the browser cannot provide', async () => {
    // Returning a default would let a contract take a branch it would never
    // take against the real host, which is worse than refusing to run it.
    const costs = { ...DEFAULT_GAS_COSTS };
    const result = await executeMetered(fixtureBytes(), 'log_sum', [1, 1], {
      costs,
    });
    expect(result.success).toBe(true); // host_log *is* supported

    // Confirm the failure path is wired for an import the mock does not know.
    const unsupported = await executeMetered(
      unknownImportModule(),
      'callIt',
      [],
    );
    expect(unsupported.success).toBe(false);
    expect(unsupported.errorCode).toBe('unsupported_host_fn');
    expect(unsupported.errorMessage).toContain('mystery_fn');
  });

  it('always reports a duration', async () => {
    const result = await executeMetered(fixtureBytes(), 'add', [1, 2]);
    expect(result.durationMs).toBeGreaterThanOrEqual(0);
  });
});

/**
 * Fixture importing `env.mystery_fn`, which the mock host does not implement.
 * Built here rather than checked in so the two fixtures stay readable.
 */
function unknownImportModule(): Uint8Array<ArrayBuffer> {
  // (module (import "env" "mystery_fn" (func $m)) (func (export "callIt") call $m))
  const b64 =
    'AGFzbQEAAAABBAFgAAACEgEDZW52Cm15c3RlcnlfZm4AAAMCAQAHCgEGY2FsbEl0AAEKBgEEABAACw==';
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

describe('BrowserWasmRunner', () => {
  /** Worker stand-in that runs the request synchronously in-process. */
  function fakeWorker(): SandboxWorkerLike & { terminated: boolean } {
    const worker: SandboxWorkerLike & { terminated: boolean } = {
      terminated: false,
      onmessage: null,
      onerror: null,
      postMessage(request: SandboxRequest) {
        void executeMetered(
          new Uint8Array(request.wasm),
          request.fn,
          request.args,
          { gasLimit: request.gasLimit, costs: request.costs },
        ).then((result) => {
          const response: SandboxResponse = { id: request.id, result };
          worker.onmessage?.({ data: response });
        });
      },
      terminate() {
        worker.terminated = true;
      },
    };
    return worker;
  }

  const wasmBuffer = () => {
    const bytes = fixtureBytes();
    return bytes.buffer.slice(0) as ArrayBuffer;
  };

  it('returns the worker result', async () => {
    const runner = new BrowserWasmRunner(undefined, undefined, fakeWorker);
    const result = await runner.run(wasmBuffer(), 'add', [7, 8]);

    expect(result.success).toBe(true);
    expect(result.returnValue).toBe('15');
  });

  it('reuses one worker across runs', async () => {
    const factory = vi.fn(fakeWorker);
    const runner = new BrowserWasmRunner(undefined, undefined, factory);

    await runner.run(wasmBuffer(), 'add', [1, 1]);
    await runner.run(wasmBuffer(), 'add', [2, 2]);

    expect(factory).toHaveBeenCalledTimes(1);
  });

  it('ignores a response for a different run', async () => {
    const stale: SandboxWorkerLike = {
      onmessage: null,
      onerror: null,
      postMessage(request) {
        // Reply with the wrong id first, then the right one.
        this.onmessage?.({
          data: {
            id: request.id + 99,
            result: {
              success: true,
              returnValue: 'stale',
              gasConsumed: 0,
              hostCalls: 0,
              durationMs: 0,
              traceLogs: [],
            },
          },
        });
        this.onmessage?.({
          data: {
            id: request.id,
            result: {
              success: true,
              returnValue: 'fresh',
              gasConsumed: 0,
              hostCalls: 0,
              durationMs: 0,
              traceLogs: [],
            },
          },
        });
      },
      terminate() {},
    };

    const runner = new BrowserWasmRunner(undefined, undefined, () => stale);
    const result = await runner.run(wasmBuffer(), 'add', []);

    expect(result.returnValue).toBe('fresh');
  });

  it('terminates the worker when a run exceeds its time budget', async () => {
    // A contract can loop forever and there is no way to interrupt a running
    // instance, so killing the worker is the only exit.
    const silent = fakeWorker();
    silent.postMessage = () => {};

    const runner = new BrowserWasmRunner(
      { gasLimit: 1_000, timeoutMs: 20 },
      undefined,
      () => silent,
    );
    const result = await runner.run(wasmBuffer(), 'spin', []);

    expect(result.success).toBe(false);
    expect(result.errorCode).toBe('timeout');
    expect(silent.terminated).toBe(true);
  });

  it('starts a fresh worker after one was terminated', async () => {
    const factory = vi.fn(() => {
      const w = fakeWorker();
      return w;
    });
    const runner = new BrowserWasmRunner(undefined, undefined, factory);

    await runner.run(wasmBuffer(), 'add', [1, 1]);
    runner.terminate();
    await runner.run(wasmBuffer(), 'add', [1, 1]);

    expect(factory).toHaveBeenCalledTimes(2);
  });

  it('surfaces a worker crash as a trap rather than hanging', async () => {
    const crashing: SandboxWorkerLike = {
      onmessage: null,
      onerror: null,
      postMessage() {
        this.onerror?.({ message: 'boom' });
      },
      terminate() {},
    };

    const runner = new BrowserWasmRunner(undefined, undefined, () => crashing);
    const result = await runner.run(wasmBuffer(), 'add', []);

    expect(result.success).toBe(false);
    expect(result.errorCode).toBe('trap');
    expect(result.errorMessage).toBe('boom');
  });
});

describe('backend parity', () => {
  it('reports the same result shape the backend runner returns', async () => {
    // Parity is what makes a local dry run trustworthy: the UI must not be
    // able to tell which runner produced a result.
    const result = await executeMetered(fixtureBytes(), 'log_sum', [1, 2]);

    expect(Object.keys(result).sort()).toEqual(
      [
        'durationMs',
        'gasConsumed',
        'hostCalls',
        'returnValue',
        'success',
        'traceLogs',
      ].sort(),
    );
  });

  it('prices identical calls identically across runs', async () => {
    const a = await executeMetered(fixtureBytes(), 'log_sum', [1, 2]);
    const b = await executeMetered(fixtureBytes(), 'log_sum', [1, 2]);
    expect(a.gasConsumed).toBe(b.gasConsumed);
  });
});
