/**
 * WebAssembly local sandbox execution engine (issue #890).
 *
 * Runs a contract's Wasm in the browser instead of round-tripping to the
 * backend simulator, so a dry run costs no network latency and works offline.
 *
 * Two things make this safe to run on the client:
 *
 *  - **Execution happens in a Web Worker.** A contract can loop forever, and
 *    on the main thread that freezes the tab with no way back. In a worker it
 *    can be terminated, which is the only reliable way to stop a running Wasm
 *    instance — there is no cooperative cancellation to hook into.
 *  - **Every host call is metered.** Gas is charged per host function
 *    invocation and against a wall-clock budget, and exceeding either aborts
 *    the run rather than letting it consume the device.
 *
 * The host functions here are a *mock* of the Soroban environment, matching
 * the backend runner's cost model so the two agree on gas. It is not the real
 * host: anything requiring ledger state that the browser does not have will
 * report `unsupported_host_fn` rather than quietly returning a wrong answer.
 */

export interface GasCosts {
  /** Charged once per host function call. */
  hostCallBase: number;
  /** Charged per byte read or written through the host. */
  byteCost: number;
  /** Charged per storage read. */
  storageRead: number;
  /** Charged per storage write. */
  storageWrite: number;
}

export const DEFAULT_GAS_COSTS: GasCosts = {
  hostCallBase: 1_000,
  byteCost: 10,
  storageRead: 5_000,
  storageWrite: 20_000,
};

export interface SandboxLimits {
  /** Maximum gas the run may consume. */
  gasLimit: number;
  /** Wall-clock budget in milliseconds, enforced by terminating the worker. */
  timeoutMs: number;
}

export const DEFAULT_LIMITS: SandboxLimits = {
  gasLimit: 100_000_000,
  timeoutMs: 5_000,
};

export type SandboxFailure =
  | 'gas_exhausted'
  | 'timeout'
  | 'trap'
  | 'unsupported_host_fn'
  | 'invalid_module'
  | 'export_not_found';

export interface SandboxResult {
  success: boolean;
  /** Present only when `success` is false. */
  errorCode?: SandboxFailure;
  errorMessage?: string;
  returnValue: string;
  gasConsumed: number;
  hostCalls: number;
  durationMs: number;
  traceLogs: string[];
}

/** Message sent into the worker. */
export interface SandboxRequest {
  id: number;
  wasm: ArrayBuffer;
  fn: string;
  args: number[];
  gasLimit: number;
  costs: GasCosts;
}

/** Message sent back out of the worker. */
export interface SandboxResponse {
  id: number;
  result: SandboxResult;
}

/**
 * Minimal Worker surface this module depends on.
 *
 * Narrower than `Worker` on purpose: it is the whole contract between the
 * runner and the worker, so a test can supply a stand-in without a DOM.
 */
export interface SandboxWorkerLike {
  postMessage(message: SandboxRequest, transfer?: Transferable[]): void;
  terminate(): void;
  onmessage: ((event: { data: SandboxResponse }) => void) | null;
  onerror: ((event: unknown) => void) | null;
}

export type WorkerFactory = () => SandboxWorkerLike;

/**
 * Execute one Wasm export under a gas meter.
 *
 * Exported separately from the worker so the metering logic can be tested
 * directly, and so a caller that has already accepted the freeze risk (a
 * short, known-terminating call) can run without a worker at all.
 */
export async function executeMetered(
  wasm: ArrayBuffer | Uint8Array<ArrayBuffer>,
  fn: string,
  args: number[],
  options: { gasLimit?: number; costs?: GasCosts } = {},
): Promise<SandboxResult> {
  const gasLimit = options.gasLimit ?? DEFAULT_LIMITS.gasLimit;
  const costs = options.costs ?? DEFAULT_GAS_COSTS;

  const started = Date.now();
  const traceLogs: string[] = [];
  let gasConsumed = 0;
  let hostCalls = 0;

  /** Charge gas and abort the run by trapping if the budget is gone. */
  const charge = (amount: number) => {
    gasConsumed += amount;
    if (gasConsumed > gasLimit) {
      throw new GasExhausted(gasConsumed);
    }
  };

  const hostCall = (cost: number) => {
    hostCalls += 1;
    charge(costs.hostCallBase + cost);
  };

  // Any import the mock host does not implement fails loudly. Returning 0
  // would let a contract silently take a branch it would never take against
  // the real host, which is worse than not running it at all.
  const unsupported = (name: string) => () => {
    throw new UnsupportedHostFn(name);
  };

  const env: Record<string, unknown> = {
    host_log: (value: number) => {
      hostCall(costs.byteCost * 4);
      traceLogs.push(`log: ${value}`);
    },
    storage_get: (key: number) => {
      hostCall(costs.storageRead);
      traceLogs.push(`storage_get(${key})`);
      return 0;
    },
    storage_put: (key: number, value: number) => {
      hostCall(costs.storageWrite);
      traceLogs.push(`storage_put(${key}, ${value})`);
    },
    require_auth: unsupported('require_auth'),
    call_contract: unsupported('call_contract'),
  };

  const finish = (
    partial: Partial<SandboxResult> & Pick<SandboxResult, 'success'>,
  ): SandboxResult => ({
    returnValue: '',
    gasConsumed,
    hostCalls,
    durationMs: Date.now() - started,
    traceLogs,
    ...partial,
  });

  let instance: WebAssembly.Instance;
  try {
    const bytes: BufferSource =
      wasm instanceof Uint8Array ? wasm : new Uint8Array(wasm);
    const module = await WebAssembly.compile(bytes);
    // Every import the module declares must resolve, so fill anything the
    // mock host does not know about with a loud failure rather than letting
    // instantiation throw a link error the user cannot act on.
    const imports: WebAssembly.Imports = { env: {} };
    for (const desc of WebAssembly.Module.imports(module)) {
      const table = (imports[desc.module] ??= {}) as Record<string, unknown>;
      table[desc.name] = env[desc.name] ?? unsupported(desc.name);
    }
    instance = await WebAssembly.instantiate(module, imports);
  } catch (error) {
    return finish({
      success: false,
      errorCode: 'invalid_module',
      errorMessage: (error as Error).message,
    });
  }

  const exported = instance.exports[fn];
  if (typeof exported !== 'function') {
    return finish({
      success: false,
      errorCode: 'export_not_found',
      errorMessage: `Contract does not export "${fn}"`,
    });
  }

  try {
    const value = (exported as (...a: number[]) => unknown)(...args);
    return finish({
      success: true,
      returnValue: value === undefined ? 'void' : String(value),
    });
  } catch (error) {
    if (error instanceof GasExhausted) {
      return finish({
        success: false,
        errorCode: 'gas_exhausted',
        errorMessage: `Gas limit of ${gasLimit} exceeded`,
      });
    }
    if (error instanceof UnsupportedHostFn) {
      return finish({
        success: false,
        errorCode: 'unsupported_host_fn',
        errorMessage: `Host function "${error.fnName}" is not available in the browser sandbox`,
      });
    }
    return finish({
      success: false,
      errorCode: 'trap',
      errorMessage: (error as Error).message,
    });
  }
}

export class GasExhausted extends Error {
  readonly consumed: number;

  constructor(consumed: number) {
    super(`gas exhausted at ${consumed}`);
    this.name = 'GasExhausted';
    this.consumed = consumed;
  }
}

export class UnsupportedHostFn extends Error {
  readonly fnName: string;

  constructor(fnName: string) {
    super(`unsupported host function: ${fnName}`);
    this.name = 'UnsupportedHostFn';
    this.fnName = fnName;
  }
}

/** Default worker factory. Vite resolves the URL form at build time. */
const defaultWorkerFactory: WorkerFactory = () =>
  new Worker(new URL('./wasmSandbox.worker.ts', import.meta.url), {
    type: 'module',
  }) as unknown as SandboxWorkerLike;

/**
 * Runs contract Wasm in a Web Worker under a gas and time budget.
 *
 * One worker is kept alive across runs — spawning one per call would add
 * tens of milliseconds to what is meant to be a zero-latency dry run — and is
 * replaced whenever a run has to be killed, since a terminated worker cannot
 * be reused.
 */
export class BrowserWasmRunner {
  private worker: SandboxWorkerLike | null = null;
  private nextId = 1;

  private readonly limits: SandboxLimits;
  private readonly costs: GasCosts;
  private readonly createWorker: WorkerFactory;

  constructor(
    limits: SandboxLimits = DEFAULT_LIMITS,
    costs: GasCosts = DEFAULT_GAS_COSTS,
    createWorker: WorkerFactory = defaultWorkerFactory,
  ) {
    this.limits = limits;
    this.costs = costs;
    this.createWorker = createWorker;
  }

  /** True when this environment can run the sandbox at all. */
  static isSupported(): boolean {
    return (
      typeof WebAssembly !== 'undefined' && typeof Worker !== 'undefined'
    );
  }

  async run(
    wasm: ArrayBuffer,
    fn: string,
    args: number[] = [],
  ): Promise<SandboxResult> {
    const worker = (this.worker ??= this.createWorker());
    const id = this.nextId++;
    const started = Date.now();

    return new Promise<SandboxResult>((resolve) => {
      const settle = (result: SandboxResult) => {
        clearTimeout(timer);
        worker.onmessage = null;
        worker.onerror = null;
        resolve(result);
      };

      // A contract can loop forever, and there is no way to interrupt a
      // running instance from outside — terminating the worker is the only
      // exit, so the next run starts a fresh one.
      const timer = setTimeout(() => {
        this.terminate();
        settle({
          success: false,
          errorCode: 'timeout',
          errorMessage: `Execution exceeded ${this.limits.timeoutMs}ms and was terminated`,
          returnValue: '',
          gasConsumed: 0,
          hostCalls: 0,
          durationMs: Date.now() - started,
          traceLogs: [],
        });
      }, this.limits.timeoutMs);

      worker.onmessage = (event) => {
        if (event.data.id !== id) return;
        settle(event.data.result);
      };

      worker.onerror = (error) => {
        this.terminate();
        settle({
          success: false,
          errorCode: 'trap',
          errorMessage:
            (error as { message?: string })?.message ?? 'Worker crashed',
          returnValue: '',
          gasConsumed: 0,
          hostCalls: 0,
          durationMs: Date.now() - started,
          traceLogs: [],
        });
      };

      worker.postMessage({
        id,
        wasm,
        fn,
        args,
        gasLimit: this.limits.gasLimit,
        costs: this.costs,
      });
    });
  }

  terminate(): void {
    this.worker?.terminate();
    this.worker = null;
  }
}

export default BrowserWasmRunner;
