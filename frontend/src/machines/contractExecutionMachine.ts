export type ExecutionState =
  | 'idle'
  | 'validating'
  | 'simulating'
  | 'signing'
  | 'submitting'
  | 'success'
  | 'error';

export type ExecutionEvent =
  | { type: 'START_EXECUTION'; payload: { functionName: string; inputs: Record<string, string> } }
  | { type: 'VALIDATED' }
  | { type: 'SIMULATION_SUCCESS'; result: any }
  | { type: 'SIGNED' }
  | { type: 'SUBMISSION_SUCCESS'; result: any }
  | { type: 'FAIL'; error: string }
  | { type: 'RESET' };

export interface ContractExecutionMachineContext {
  functionName: string;
  inputs: Record<string, string>;
  result: any | null;
  error: string | null;
}

export const initialContext: ContractExecutionMachineContext = {
  functionName: '',
  inputs: {},
  result: null,
  error: null,
};

export class ContractExecutionStateMachine {
  private currentState: ExecutionState = 'idle';
  private context: ContractExecutionMachineContext = { ...initialContext };
  private listeners: Array<(state: ExecutionState, context: ContractExecutionMachineContext) => void> = [];

  constructor(initialCtx?: Partial<ContractExecutionMachineContext>) {
    if (initialCtx) {
      this.context = { ...initialContext, ...initialCtx };
    }
  }

  public getState(): ExecutionState {
    return this.currentState;
  }

  public getContext(): ContractExecutionMachineContext {
    return { ...this.context };
  }

  public subscribe(listener: (state: ExecutionState, context: ContractExecutionMachineContext) => void) {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter(l => l !== listener);
    };
  }

  private notify() {
    this.listeners.forEach(listener => listener(this.currentState, { ...this.context }));
  }

  public send(event: ExecutionEvent): ExecutionState {
    switch (this.currentState) {
      case 'idle':
        if (event.type === 'START_EXECUTION') {
          this.currentState = 'validating';
          this.context.functionName = event.payload.functionName;
          this.context.inputs = event.payload.inputs;
          this.context.error = null;
          this.context.result = null;
        }
        break;

      case 'validating':
        if (event.type === 'VALIDATED') {
          this.currentState = 'simulating';
        } else if (event.type === 'FAIL') {
          this.currentState = 'error';
          this.context.error = event.error;
        }
        break;

      case 'simulating':
        if (event.type === 'SIMULATION_SUCCESS') {
          this.currentState = 'signing';
        } else if (event.type === 'FAIL') {
          this.currentState = 'error';
          this.context.error = event.error;
        }
        break;

      case 'signing':
        if (event.type === 'SIGNED') {
          this.currentState = 'submitting';
        } else if (event.type === 'FAIL') {
          this.currentState = 'error';
          this.context.error = event.error;
        }
        break;

      case 'submitting':
        if (event.type === 'SUBMISSION_SUCCESS') {
          this.currentState = 'success';
          this.context.result = event.result;
        } else if (event.type === 'FAIL') {
          this.currentState = 'error';
          this.context.error = event.error;
        }
        break;

      case 'success':
      case 'error':
        if (event.type === 'RESET') {
          this.currentState = 'idle';
          this.context = { ...initialContext };
        } else if (event.type === 'START_EXECUTION') {
          this.currentState = 'validating';
          this.context.functionName = event.payload.functionName;
          this.context.inputs = event.payload.inputs;
          this.context.error = null;
          this.context.result = null;
        }
        break;
    }

    this.notify();
    return this.currentState;
  }
}
