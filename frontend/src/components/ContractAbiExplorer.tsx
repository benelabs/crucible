import React, { useState } from 'react';
import { Cpu, Play, HelpCircle, Code2, FileText, CheckCircle2 } from 'lucide-react';
import './ContractAbiExplorer.css';

interface FunctionArg {
  name: string;
  type: string;
}

interface ContractFunction {
  name: string;
  args: FunctionArg[];
  returnType: string;
}

interface ContractAbi {
  name: string;
  functions: ContractFunction[];
}

const CONTRACT_ABIS: ContractAbi[] = [
  {
    name: 'Counter',
    functions: [
      { name: 'increment', args: [], returnType: 'u32' },
      { name: 'decrement', args: [], returnType: 'u32' },
      { name: 'get_value', args: [], returnType: 'u32' },
      { name: 'reset', args: [{ name: 'to', type: 'u32' }], returnType: 'void' },
    ],
  },
  {
    name: 'Token',
    functions: [
      {
        name: 'initialize',
        args: [
          { name: 'admin', type: 'Address' },
          { name: 'name', type: 'String' },
          { name: 'symbol', type: 'String' },
        ],
        returnType: 'void',
      },
      { name: 'balance', args: [{ name: 'user', type: 'Address' }], returnType: 'u128' },
      {
        name: 'transfer',
        args: [
          { name: 'from', type: 'Address' },
          { name: 'to', type: 'Address' },
          { name: 'amount', type: 'u128' },
        ],
        returnType: 'void',
      },
      {
        name: 'mint',
        args: [
          { name: 'to', type: 'Address' },
          { name: 'amount', type: 'u128' },
        ],
        returnType: 'void',
      },
    ],
  },
  {
    name: 'Vault',
    functions: [
      {
        name: 'deposit',
        args: [
          { name: 'user', type: 'Address' },
          { name: 'amount', type: 'u128' },
        ],
        returnType: 'u128',
      },
      {
        name: 'withdraw',
        args: [
          { name: 'user', type: 'Address' },
          { name: 'amount', type: 'u128' },
        ],
        returnType: 'u128',
      },
      { name: 'get_shares', args: [{ name: 'user', type: 'Address' }], returnType: 'u128' },
    ],
  },
];

export const ContractAbiExplorer: React.FC = () => {
  const [selectedContractIndex, setSelectedContractIndex] = useState<number>(0);
  const [selectedFuncIndex, setSelectedFuncIndex] = useState<number>(0);
  const [inputs, setInputs] = useState<Record<string, string>>({});
  const [executionResult, setExecutionResult] = useState<any>(null);
  const [isRunning, setIsRunning] = useState<boolean>(false);

  const contract = CONTRACT_ABIS[selectedContractIndex];
  const func = contract.functions[selectedFuncIndex] || contract.functions[0];

  const handleInputChange = (argName: string, value: string) => {
    setInputs(prev => ({
      ...prev,
      [argName]: value,
    }));
  };

  const handleRunExecution = () => {
    setIsRunning(true);
    setExecutionResult(null);

    // Simulate function execution
    setTimeout(() => {
      setIsRunning(false);

      // Generate a plausible mock response based on returnType and input values
      let returnValue = 'void';
      let gasCost = Math.floor(Math.random() * 4000) + 1200;

      if (func.returnType === 'u32') {
        if (func.name === 'get_value') {
          returnValue = '42';
        } else {
          returnValue = String(Math.floor(Math.random() * 100) + 1);
        }
      } else if (func.returnType === 'u128') {
        const amt = inputs['amount'] || '1000';
        if (func.name === 'deposit' || func.name === 'withdraw') {
          returnValue = String(parseInt(amt) * 98 / 100); // 2% fee / share ratio
        } else {
          returnValue = String(Math.floor(Math.random() * 10000) + 500);
        }
      }

      setExecutionResult({
        status: 'success',
        returnValue,
        gasCost,
        events: [
          {
            name: `${func.name}_invoked`,
            data: JSON.stringify(inputs),
          },
        ],
        ledgerHeight: 452936,
      });
    }, 900);
  };

  const handleSelectContract = (idx: number) => {
    setSelectedContractIndex(idx);
    setSelectedFuncIndex(0);
    setInputs({});
    setExecutionResult(null);
  };

  const handleSelectFunc = (idx: number) => {
    setSelectedFuncIndex(idx);
    setInputs({});
    setExecutionResult(null);
  };

  return (
    <div className="abi-explorer-container">
      <div className="abi-header">
        <div className="header-icon-wrapper">
          <Cpu className="header-icon" />
        </div>
        <div>
          <h2>Contract ABI Explorer</h2>
          <p>Inspect smart contract interface definition and run client-side function simulation</p>
        </div>
      </div>

      <div className="abi-content">
        <div className="contracts-sidebar glass-panel">
          <h3 className="section-title">Select Contract</h3>
          <div className="contracts-list">
            {CONTRACT_ABIS.map((c, idx) => (
              <button
                key={c.name}
                className={`contract-item-btn ${selectedContractIndex === idx ? 'active' : ''}`}
                onClick={() => handleSelectContract(idx)}
                data-testid={`abi-select-${c.name.toLowerCase()}`}
              >
                <FileText size={16} />
                {c.name}
              </button>
            ))}
          </div>

          <h3 className="section-title" style={{ marginTop: '24px' }}>Functions / Methods</h3>
          <div className="methods-list">
            {contract.functions.map((f, idx) => (
              <button
                key={f.name}
                className={`method-item-btn ${selectedFuncIndex === idx ? 'active' : ''}`}
                onClick={() => handleSelectFunc(idx)}
                data-testid={`method-${f.name}`}
              >
                <Code2 size={14} />
                <span className="method-name">{f.name}</span>
                <span className="method-type-badge">{f.returnType}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="function-testing-panel glass-panel" data-testid="abi-testing-panel">
          <div className="panel-header">
            <h3>Test Function: <span className="highlight">{func.name}</span></h3>
          </div>

          <div className="function-signature">
            <span className="sig-fn">fn</span> <span className="sig-name">{func.name}</span>(
            {func.args.map((a, i) => (
              <span key={a.name}>
                <span className="sig-arg">{a.name}</span>: <span className="sig-type">{a.type}</span>
                {i < func.args.length - 1 ? ', ' : ''}
              </span>
            ))}
            ) -&gt; <span className="sig-return">{func.returnType}</span>
          </div>

          <div className="function-inputs">
            {func.args.length === 0 ? (
              <div className="no-args-message">
                <HelpCircle size={16} />
                No parameters required for this function.
              </div>
            ) : (
              <div className="inputs-grid">
                {func.args.map(arg => (
                  <div className="input-group" key={arg.name}>
                    <label htmlFor={`input-${arg.name}`}>
                      {arg.name} <span className="arg-type">({arg.type})</span>
                    </label>
                    <input
                      id={`input-${arg.name}`}
                      type={arg.type === 'u32' || arg.type === 'u128' ? 'number' : 'text'}
                      placeholder={`Enter ${arg.type}`}
                      value={inputs[arg.name] || ''}
                      onChange={e => handleInputChange(arg.name, e.target.value)}
                      data-testid={`input-${arg.name}`}
                    />
                  </div>
                ))}
              </div>
            )}
          </div>

          <button
            className={`execute-btn ${isRunning ? 'running' : ''}`}
            onClick={handleRunExecution}
            disabled={isRunning}
            data-testid="execute-btn"
          >
            {isRunning ? 'Executing Simulation...' : 'Simulate Call'}
            {!isRunning && <Play size={14} />}
          </button>

          {executionResult && (
            <div className="execution-result-block" data-testid="execution-result">
              <div className="result-header">
                <CheckCircle2 size={16} className="success-icon" />
                <h4>Simulation Output</h4>
              </div>
              <div className="result-stats">
                <div className="res-stat-card">
                  <span className="stat-lbl">Return Value</span>
                  <span className="stat-val highlight">{executionResult.returnValue}</span>
                </div>
                <div className="res-stat-card">
                  <span className="stat-lbl">Gas Expended</span>
                  <span className="stat-val">{executionResult.gasCost.toLocaleString()} stroops</span>
                </div>
                <div className="res-stat-card">
                  <span className="stat-lbl">Ledger Reference</span>
                  <span className="stat-val">#{executionResult.ledgerHeight}</span>
                </div>
              </div>

              {executionResult.events && executionResult.events.length > 0 && (
                <div className="events-block">
                  <h5>Emitted Events</h5>
                  {executionResult.events.map((evt: any, i: number) => (
                    <div className="event-row" key={i}>
                      <span className="event-name">[{evt.name}]</span>
                      <span className="event-data">{evt.data}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};
