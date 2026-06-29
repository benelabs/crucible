import React, { useState, useEffect, useRef } from 'react';
import { 
  Cpu, 
  Database, 
  Coins, 
  Terminal, 
  ArrowRight, 
  RotateCcw, 
  Settings, 
  ShieldAlert, 
  CheckCircle2, 
  Play,
  HelpCircle,
  Activity,
  Layers,
  ChevronDown
} from 'lucide-react';
import { 
  useTransactionSimulator, 
  CONTRACT_TEMPLATES, 
  MOCK_ACCOUNTS
} from './useTransactionSimulator';
import './TransactionSimulator.css';

export const TransactionSimulator: React.FC = () => {
  const {
    selectedContractId,
    selectedContract,
    selectedFunctionName,
    selectedFunction,
    args,
    authSignatures,
    envConfig,
    isSimulating,
    simulationResult,
    handleSelectContract,
    handleSelectFunction,
    handleArgChange,
    toggleAuthSignature,
    handleEnvChange,
    resetConfig,
    runSimulation,
  } = useTransactionSimulator();

  const [isEnvOpen, setIsEnvOpen] = useState(false);
  const consoleEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll VM console when traceLogs are appended
  useEffect(() => {
    if (consoleEndRef.current && typeof consoleEndRef.current.scrollIntoView === 'function') {
      consoleEndRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [simulationResult?.traceLogs]);

  // Determine CPU Instruction level color class
  const getCpuColorClass = (consumed: number, limit: number) => {
    const ratio = consumed / limit;
    if (ratio > 0.8) return 'red';
    if (ratio > 0.4) return 'yellow';
    return 'green';
  };

  return (
    <div className="sim-dashboard" data-testid="sim-dashboard">
      <div className="sim-dashboard__header">
        <div className="sim-dashboard__title-block">
          <div className="sim-dashboard__icon">
            <Cpu size={22} />
          </div>
          <div>
            <h2>Transaction Simulator</h2>
            <p>Dry-run smart contract calls with low-level execution insights</p>
          </div>
        </div>

        <button 
          type="button" 
          className="reset-all-btn" 
          onClick={resetConfig}
          title="Reset configuration to defaults"
          data-testid="reset-btn"
        >
          <RotateCcw size={15} />
          Reset Settings
        </button>
      </div>

      <div className="sim-dashboard__grid">
        {/* CONFIGURATION COLUMN */}
        <div className="sim-dashboard__config glass-panel">
          <h3 className="section-title">Contract & Invocation</h3>

          {/* Contract Selector */}
          <div className="form-group">
            <label>Target Contract</label>
            <div className="contract-selector" role="group" aria-label="Select Target Contract">
              {CONTRACT_TEMPLATES.map((c) => (
                <button
                  key={c.id}
                  type="button"
                  className={`contract-btn ${selectedContractId === c.id ? 'active' : ''}`}
                  onClick={() => handleSelectContract(c.id)}
                  data-testid={`contract-select-${c.id}`}
                >
                  {c.name}
                </button>
              ))}
            </div>
          </div>

          {/* Function Selector */}
          <div className="form-group">
            <label>Function Call</label>
            <div className="function-selector" role="group" aria-label="Select Function Call">
              {selectedContract.functions.map((f) => (
                <button
                  key={f.name}
                  type="button"
                  className={`function-btn ${selectedFunctionName === f.name ? 'active' : ''}`}
                  onClick={() => handleSelectFunction(f.name)}
                  data-testid={`function-select-${f.name}`}
                >
                  {f.name}()
                </button>
              ))}
            </div>
            <p className="input-desc" style={{ marginTop: '2px', color: '#94a3b8' }}>
              {selectedFunction.description}
            </p>
          </div>

          {/* Dynamic Arguments Input Fields */}
          <div style={{ marginTop: '24px', marginBottom: '24px' }}>
            <h4 style={{ fontSize: '0.875rem', fontWeight: 600, color: '#f1f5f9', marginBottom: '16px' }}>
              Arguments
            </h4>
            
            {selectedFunction.arguments.map((arg) => (
              <div className="form-group" key={arg.name}>
                <label htmlFor={`arg-input-${arg.name}`}>
                  {arg.name} <span>{arg.type}</span>
                </label>

                {arg.type === 'address' ? (
                  <select
                    id={`arg-input-${arg.name}`}
                    value={args[arg.name] || ''}
                    onChange={(e) => handleArgChange(arg.name, e.target.value)}
                    className="text-input"
                    data-testid={`arg-field-${arg.name}`}
                  >
                    {MOCK_ACCOUNTS.map((acc) => (
                      <option key={acc.name} value={acc.name}>
                        {acc.name} ({acc.address.slice(0, 8)}...)
                      </option>
                    ))}
                    <option value="custom">Custom Address...</option>
                  </select>
                ) : (
                  <input
                    id={`arg-input-${arg.name}`}
                    type={arg.type === 'u32' ? 'number' : 'text'}
                    value={args[arg.name] || ''}
                    placeholder={arg.placeholder}
                    onChange={(e) => handleArgChange(arg.name, e.target.value)}
                    className="text-input"
                    data-testid={`arg-field-${arg.name}`}
                  />
                )}
                
                <p className="input-desc">{arg.description}</p>
              </div>
            ))}
          </div>

          {/* Authorization Checkbox Cards */}
          <div style={{ marginBottom: '24px' }}>
            <h4 style={{ fontSize: '0.875rem', fontWeight: 600, color: '#f1f5f9', marginBottom: '8px' }}>
              Authorization Signatures
            </h4>
            <p className="input-desc" style={{ marginBottom: '14px' }}>
              Check to verify sign-off by simulated keypairs. Unchecking triggers authorization reverts.
            </p>

            <div className="auth-grid">
              {MOCK_ACCOUNTS.map((acc) => (
                <button
                  key={acc.name}
                  type="button"
                  onClick={() => toggleAuthSignature(acc.name)}
                  className={`auth-checkbox-card ${authSignatures[acc.name] ? 'checked' : ''}`}
                  data-testid={`auth-card-${acc.name}`}
                >
                  <span className="checkbox-custom" aria-hidden="true" />
                  <div className="auth-info">
                    <span className="auth-name">{acc.name}</span>
                    <span className="auth-desc">{acc.description}</span>
                  </div>
                </button>
              ))}
            </div>
          </div>

          {/* Advanced Environment Overrides Accordion */}
          <div className={`accordion ${isEnvOpen ? 'open' : ''}`} data-testid="env-accordion">
            <button
              type="button"
              className="accordion__header"
              onClick={() => setIsEnvOpen(!isEnvOpen)}
              data-testid="env-accordion-header"
            >
              <h4>
                <Settings size={15} />
                Ledger & VM State Overrides
              </h4>
              <ChevronDown className="accordion__icon" size={15} />
            </button>

            {isEnvOpen && (
              <div className="accordion__content">
                <div className="form-group">
                  <label htmlFor="env-seq">
                    Ledger Sequence <span>{envConfig.ledgerSeq.toLocaleString()}</span>
                  </label>
                  <input
                    id="env-seq"
                    type="range"
                    min="1000000"
                    max="20000000"
                    step="1000"
                    value={envConfig.ledgerSeq}
                    onChange={(e) => handleEnvChange('ledgerSeq', parseInt(e.target.value))}
                    className="range-slider"
                    data-testid="env-seq-slider"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="env-timestamp">
                    Timestamp (UNIX) <span>{envConfig.ledgerTimestamp}</span>
                  </label>
                  <input
                    id="env-timestamp"
                    type="number"
                    value={envConfig.ledgerTimestamp}
                    onChange={(e) => handleEnvChange('ledgerTimestamp', parseInt(e.target.value) || 0)}
                    className="text-input"
                    data-testid="env-timestamp-input"
                  />
                </div>

                <div className="form-group">
                  <label htmlFor="env-fee">
                    Base Operation Fee <span>{envConfig.feeRate} Stroops</span>
                  </label>
                  <input
                    id="env-fee"
                    type="number"
                    value={envConfig.feeRate}
                    onChange={(e) => handleEnvChange('feeRate', parseInt(e.target.value) || 0)}
                    className="text-input"
                    data-testid="env-fee-input"
                  />
                </div>
              </div>
            )}
          </div>

          {/* Sim Execution Button */}
          <button
            type="button"
            className="simulate-big-btn"
            onClick={runSimulation}
            disabled={isSimulating}
            data-testid="run-sim-btn"
          >
            {isSimulating ? (
              <>
                <span className="spinner" aria-hidden="true" />
                Simulating Invocation...
              </>
            ) : (
              <>
                <Play size={16} fill="white" />
                Simulate Transaction
                <ArrowRight size={16} />
              </>
            )}
          </button>
        </div>

        {/* SIMULATION RESULTS COLUMN */}
        <div className="sim-dashboard__results">
          {!simulationResult && !isSimulating ? (
            <div className="empty-results-panel glass-panel" data-testid="empty-results">
              <div className="empty-icon-wrapper">
                <Terminal size={24} />
              </div>
              <h3>Awaiting Simulation</h3>
              <p>Configure parameters on the left and click "Simulate Transaction" to invoke the virtual environment.</p>
            </div>
          ) : isSimulating ? (
            <div className="empty-results-panel glass-panel" data-testid="simulating-results">
              <div className="spinner" style={{ width: '32px', height: '32px', marginBottom: '20px', borderTopColor: '#06b6d4' }} />
              <h3>Running Dry-Run Context</h3>
              <p>Host VM is parsing bytecode, constructing instance references, and checking authorizations...</p>
            </div>
          ) : simulationResult ? (
            <div className="glass-panel" data-testid="simulation-content" style={{ animation: 'simFadeIn 0.3s ease-out' }}>
              {/* Header Status Bar */}
              <div className="results-header-row">
                <div className="results-title">
                  <Activity size={18} className="text-purple-400" />
                  <h4>Execution Summary</h4>
                </div>
                <div 
                  className={`results-status-badge ${simulationResult.success ? 'success' : 'reverted'}`}
                  data-testid="sim-status-badge"
                >
                  {simulationResult.success ? (
                    <>
                      <CheckCircle2 size={14} />
                      Success
                    </>
                  ) : (
                    <>
                      <ShieldAlert size={14} />
                      Reverted
                    </>
                  )}
                </div>
              </div>

              {/* Error Details Banner */}
              {!simulationResult.success && (
                <div className="error-banner" data-testid="error-banner">
                  <ShieldAlert className="error-banner__icon" size={20} />
                  <div className="error-banner__details">
                    <h5>{simulationResult.errorCode}</h5>
                    <p>{simulationResult.errorMessage}</p>
                  </div>
                </div>
              )}

              {/* Stats Cards */}
              <div className="metrics-row">
                <div className="metric-card cyan">
                  <div className="metric-card__icon">
                    <Database size={16} />
                  </div>
                  <div className="metric-card__val">
                    <span>Memory Footprint</span>
                    <strong data-testid="metric-ram">{simulationResult.ramConsumed} KB</strong>
                  </div>
                </div>

                <div className="metric-card purple">
                  <div className="metric-card__icon">
                    <Coins size={16} />
                  </div>
                  <div className="metric-card__val">
                    <span>Estimated Fee</span>
                    <strong data-testid="metric-fee">{simulationResult.estimatedFee.toLocaleString()} Stroops</strong>
                  </div>
                </div>
              </div>

              {/* CPU instruction progress meter */}
              <div className="gauge-section">
                <div className="gauge-labels">
                  <span>CPU INSTRUCTIONS</span>
                  <span data-testid="gauge-val">
                    {simulationResult.cpuConsumed.toLocaleString()} / {envConfig.cpuLimit.toLocaleString()}
                  </span>
                </div>
                <div className="gauge-track">
                  <div 
                    className={`gauge-bar ${getCpuColorClass(simulationResult.cpuConsumed, envConfig.cpuLimit)}`} 
                    style={{ width: `${Math.min(100, (simulationResult.cpuConsumed / envConfig.cpuLimit) * 100)}%` }}
                    data-testid="cpu-gauge"
                  />
                </div>
              </div>

              {/* Returned Value Panel */}
              <div className="footprint-box" style={{ marginBottom: '20px' }}>
                <div className="footprint-title">
                  <Layers size={14} />
                  Output Return Value
                </div>
                <div 
                  className="footprint-key-tag" 
                  style={{ 
                    fontFamily: 'monospace', 
                    fontSize: '0.875rem', 
                    padding: '8px 12px', 
                    color: simulationResult.success ? '#4ade80' : '#f87171',
                    background: '#020617',
                    borderLeft: `2px solid ${simulationResult.success ? '#10b981' : '#ef4444'}`
                  }}
                  data-testid="return-val"
                >
                  {simulationResult.success ? simulationResult.returnValue : 'reverted'}
                </div>
              </div>

              {/* Storage Footprint Keys */}
              <div className="footprint-box">
                <div className="footprint-title">
                  <Database size={14} />
                  Ledger Storage Footprint
                </div>
                <div className="footprint-lists">
                  <div className="footprint-list">
                    <h5>Read Keys ({simulationResult.readKeys.length})</h5>
                    {simulationResult.readKeys.length > 0 ? (
                      <ul>
                        {simulationResult.readKeys.map((k) => (
                          <li key={k}>
                            <span className="footprint-key-tag read" title={k}>{k}</span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <span className="footprint-empty-msg">No keys read</span>
                    )}
                  </div>

                  <div className="footprint-list">
                    <h5>Write Keys ({simulationResult.writeKeys.length})</h5>
                    {simulationResult.writeKeys.length > 0 ? (
                      <ul>
                        {simulationResult.writeKeys.map((k) => (
                          <li key={k}>
                            <span className="footprint-key-tag write" title={k}>{k}</span>
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <span className="footprint-empty-msg" data-testid="write-keys-empty">No keys written</span>
                    )}
                  </div>
                </div>
              </div>

              {/* Contract Emitted Events */}
              <div className="sim-events-box">
                <div className="sim-events-title">
                  <HelpCircle size={14} />
                  Emitted Events ({simulationResult.emittedEvents.length})
                </div>
                
                {simulationResult.emittedEvents.length > 0 ? (
                  <div className="sim-events-list">
                    {simulationResult.emittedEvents.map((ev, i) => (
                      <div className="sim-event-item" key={i}>
                        <div className="sim-event-item__header">
                          <span className="sim-event-item__name">{ev.topicLabel}</span>
                          <span className="sim-event-item__topics">[{ev.topics.join(', ')}]</span>
                        </div>
                        <span className="sim-event-item__desc">{ev.valuePreview}</span>
                      </div>
                    ))}
                  </div>
                ) : (
                  <span className="sim-events-empty" data-testid="events-empty">
                    No contract events emitted during dry-run.
                  </span>
                )}
              </div>

              {/* Retro VM Log Console */}
              <div className="console-box">
                <div className="console-title">
                  <div className="console-title__label">
                    <Terminal size={14} />
                    VM Trace Diagnostics
                  </div>
                  <div className="console-title__label" style={{ fontSize: '0.7rem' }}>
                    <span className="console-dot" aria-hidden="true" />
                    LIVE
                  </div>
                </div>
                <div className="vm-console" data-testid="vm-console" role="log" aria-label="VM execution logs">
                  {simulationResult.traceLogs.map((log, idx) => {
                    let logType = 'info';
                    if (log.includes('[error]')) logType = 'error';
                    if (log.includes('Success!') || log.includes('successfully.')) logType = 'success';

                    return (
                      <div className={`console-line ${logType}`} key={idx}>
                        {log}
                      </div>
                    );
                  })}
                  <div ref={consoleEndRef} />
                </div>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
};
