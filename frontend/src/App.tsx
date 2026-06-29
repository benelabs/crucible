import { useState } from 'react';
import { EventListenerDashboard } from './components/EventListenerDashboard';
import { TransactionSimulator } from './components/TransactionSimulator';
import { GasCostEstimator } from './components/GasCostEstimator';
import { MultiChainDashboard } from './components/MultiChainDashboard';
import { ContractAbiExplorer } from './components/ContractAbiExplorer';
import { DeveloperOnboardingTutorial } from './components/DeveloperOnboardingTutorial';
import { Terminal, ShieldAlert, Cpu, Globe, Zap, Settings, RefreshCw, BookOpen, Radio } from 'lucide-react';
import './App.css';

type Tab = 'events' | 'tutorial' | 'metrics' | 'multichain' | 'abi' | 'compiler' | 'dependencies';
import { Terminal, ShieldAlert, Cpu, Globe, Zap, Settings, RefreshCw, Activity, Layers, BookOpen } from 'lucide-react';

type Tab = 'tutorial' | 'events' | 'simulator' | 'metrics' | 'multichain' | 'abi' | 'compiler' | 'dependencies';
import { WalletConnector } from './components/WalletConnector';
import { Terminal, ShieldAlert, Cpu, Globe, Zap, Settings, RefreshCw, BookOpen, Wallet } from 'lucide-react';

type Tab = 'tutorial' | 'metrics' | 'multichain' | 'abi' | 'compiler' | 'dependencies' | 'wallet';

function App() {
  const [activeTab, setActiveTab] = useState<Tab>('tutorial');

  // Compiler state
  const [compileProjName, setCompileProjName] = useState('my-soroban-contract');
  const [compileCode, setCompileCode] = useState(`// Paste Soroban smart contract source here\nuse soroban_sdk::{contract, contractimpl, Env};\n\n#[contract]\npub struct IncrementContract;\n\n#[contractimpl]\nimpl IncrementContract {\n    pub fn increment(env: Env) -> u32 {\n        let mut count: u32 = env.storage().instance().get(&"count").unwrap_or(0);\n        count += 1;\n        env.storage().instance().set(&"count", &count);\n        count\n    }\n}`);
  const [compiling, setCompiling] = useState(false);
  const [compileResult, setCompileResult] = useState<any>(null);

  // Dependency analyzer state
  const [cargoToml, setCargoToml] = useState(`[package]\nname = "my-soroban-contract"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\nsoroban-sdk = "25.0.0"\nserde = { version = "1.0", features = ["derive"] }\nvulnerable-crate = "0.4.2" # triggers security warning`);
  const [analyzing, setAnalyzing] = useState(false);
  const [analyzeResult, setAnalyzeResult] = useState<any>(null);

  const handleCompile = async () => {
    setCompiling(true);
    setCompileResult(null);
    try {
      const response = await fetch('http://localhost:3000/api/v1/contracts/compile', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ projectName: compileProjName, sourceCode: compileCode })
      });
      const data = await response.json();
      if (data.status === 'success') {
        setCompileResult(data.data);
      } else {
        setCompileResult({ status: 'failed', logs: data.error || 'Server compilation error' });
      }
    } catch (e: any) {
      setCompileResult({ status: 'failed', logs: `Connection error: ${e.message}` });
    } finally {
      setCompiling(false);
    }
  };

  const handleAnalyze = async () => {
    setAnalyzing(true);
    setAnalyzeResult(null);
    try {
      const response = await fetch('http://localhost:3000/api/v1/contracts/analyze-dependencies', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ cargoToml })
      });
      const data = await response.json();
      if (data.status === 'success') {
        setAnalyzeResult(data.data);
      } else {
        setAnalyzeResult({ dependencies: [], cyclesDetected: false, vulnerabilityCount: 0 });
      }
    } catch (e) {
      setAnalyzeResult({ dependencies: [], cyclesDetected: false, vulnerabilityCount: 0 });
    } finally {
      setAnalyzing(false);
    }
  };

  return (
    <div className="app-container">
      <header className="app-header">
        <div className="header-brand">
          <h1>Crucible Developer Portal</h1>
          <div className="header-badge">Soroban Toolchain</div>
        </div>
        <nav className="tab-navigation">
          <button 
            className={`nav-tab-btn ${activeTab === 'events' ? 'active' : ''}`}
            onClick={() => setActiveTab('events')}
            data-testid="tab-events"
          >
            <Radio size={15} />
            Event Listener
          </button>
          <button 
            className={`nav-tab-btn ${activeTab === 'tutorial' ? 'active' : ''}`}

        <nav className="header-tabs" aria-label="Dashboard views">
          <button
            type="button"
            className={`tab-btn ${activeTab === 'tutorial' ? 'active' : ''}`}
            onClick={() => setActiveTab('tutorial')}
            data-testid="tab-tutorial"
          >
            <BookOpen size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Tutorial
          </button>
          <button
            type="button"
            className={`tab-btn ${activeTab === 'events' ? 'active' : ''}`}
            onClick={() => setActiveTab('events')}
            data-testid="tab-events"
          >
            <Activity size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Event Listener
          </button>
          <button
            type="button"
            className={`tab-btn ${activeTab === 'simulator' ? 'active' : ''}`}
            onClick={() => setActiveTab('simulator')}
            data-testid="tab-simulator"
          >
            <Layers size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Tx Simulator
          </button>
          <button 
            type="button"
            className={`tab-btn ${activeTab === 'metrics' ? 'active' : ''}`}
            onClick={() => setActiveTab('metrics')}
            data-testid="tab-metrics"
          >
            <Zap size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Gas Estimator
          </button>
          <button 
            type="button"
            className={`tab-btn ${activeTab === 'multichain' ? 'active' : ''}`}
            onClick={() => setActiveTab('multichain')}
            data-testid="tab-multichain"
          >
            <Globe size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Node Manager
          </button>
          <button 
            type="button"
            className={`tab-btn ${activeTab === 'abi' ? 'active' : ''}`}
            onClick={() => setActiveTab('abi')}
            data-testid="tab-abi"
          >
            <Cpu size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            ABI Explorer
          </button>
          <button 
            type="button"
            className={`tab-btn ${activeTab === 'compiler' ? 'active' : ''}`}
            onClick={() => setActiveTab('compiler')}
            data-testid="tab-compiler"
          >
            <Terminal size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Compiler Service
          </button>
          <button 
            type="button"
            className={`tab-btn ${activeTab === 'dependencies' ? 'active' : ''}`}
            onClick={() => setActiveTab('dependencies')}
            data-testid="tab-dependencies"
          >
            <ShieldAlert size={15} style={{ marginRight: '6px', verticalAlign: 'middle' }} />
            Dep Analyzer
          </button>
          <button
            className={`nav-tab-btn ${activeTab === 'wallet' ? 'active' : ''}`}
            onClick={() => setActiveTab('wallet')}
            data-testid="tab-wallet"
          >
            <Wallet size={15} />
            Wallet
          </button>
        </nav>
      </header>
      
      <main className="app-main">
        {activeTab === 'events' && <EventListenerDashboard />}
        {activeTab === 'tutorial' && <DeveloperOnboardingTutorial />}
        {activeTab === 'events' && <EventListenerDashboard />}
        {activeTab === 'simulator' && <TransactionSimulator />}
        {activeTab === 'metrics' && <GasCostEstimator />}
        {activeTab === 'multichain' && <MultiChainDashboard />}
        {activeTab === 'abi' && <ContractAbiExplorer />}
        {activeTab === 'wallet' && <WalletConnector />}
        
        {activeTab === 'compiler' && (
          <div className="compiler-tab-panel container-panel">
            <div className="panel-info-header">
              <Terminal className="panel-info-icon" />
              <div>
                <h2>On-Demand compilation service</h2>
                <p>Compile Rust/Soroban smart contracts directly to WebAssembly format</p>
              </div>
            </div>

            <div className="compiler-content-grid">
              <div className="editor-side glass-panel">
                <div className="form-group-row">
                  <label htmlFor="compile-project-name">Project Name</label>
                  <input 
                    id="compile-project-name"
                    type="text" 
                    value={compileProjName}
                    onChange={e => setCompileProjName(e.target.value)}
                    className="project-name-input"
                  />
                </div>
                <div className="textarea-wrapper">
                  <label htmlFor="compile-source-code">Source Code</label>
                  <textarea 
                    id="compile-source-code"
                    value={compileCode}
                    onChange={e => setCompileCode(e.target.value)}
                    rows={15}
                    className="code-textarea"
                  />
                </div>
                <button 
                  className={`action-btn compile-run-btn ${compiling ? 'loading' : ''}`}
                  onClick={handleCompile}
                  disabled={compiling}
                  data-testid="compile-button"
                >
                  {compiling ? <RefreshCw size={15} className="spin" /> : <Terminal size={15} />}
                  {compiling ? 'Compiling contract...' : 'Compile Source'}
                </button>
              </div>

              <div className="terminal-side glass-panel" data-testid="compiler-result">
                <h3>Build Output Logs</h3>
                {compileResult ? (
                  <div className="build-output-wrapper">
                    <div className="build-metrics-row">
                      <div className="metric-pill">
                        Status: <span className={`status-text ${compileResult.status}`}>{compileResult.status}</span>
                      </div>
                      {compileResult.status === 'success' && (
                        <>
                          <div className="metric-pill">Size: {compileResult.wasmSizeBytes} B</div>
                          <div className="metric-pill">Time: {compileResult.compileTimeMs} ms</div>
                        </>
                      )}
                    </div>
                    {compileResult.status === 'success' && (
                      <div className="hash-row">
                        <span className="hash-lbl">WASM SHA256:</span>
                        <code className="hash-val">{compileResult.wasmHash}</code>
                      </div>
                    )}
                    <pre className="terminal-log-output">{compileResult.logs}</pre>
                  </div>
                ) : (
                  <div className="terminal-empty-state">
                    <Settings size={32} />
                    <p>Trigger contract compilation to inspect console logs and compilation results.</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}

        {activeTab === 'dependencies' && (
          <div className="dependencies-tab-panel container-panel">
            <div className="panel-info-header">
              <ShieldAlert className="panel-info-icon" />
              <div>
                <h2>Cargo Dependency Analyzer</h2>
                <p>Audit project configuration files for cycles, deprecated components, and vulnerability alerts</p>
              </div>
            </div>

            <div className="analyzer-content-grid">
              <div className="cargo-editor glass-panel">
                <div className="textarea-wrapper">
                  <label htmlFor="cargo-toml-content">Cargo.toml Manifest</label>
                  <textarea 
                    id="cargo-toml-content"
                    value={cargoToml}
                    onChange={e => setCargoToml(e.target.value)}
                    rows={15}
                    className="code-textarea"
                  />
                </div>
                <button 
                  className={`action-btn analyze-run-btn ${analyzing ? 'loading' : ''}`}
                  onClick={handleAnalyze}
                  disabled={analyzing}
                  data-testid="analyze-button"
                >
                  {analyzing ? <RefreshCw size={15} className="spin" /> : <ShieldAlert size={15} />}
                  {analyzing ? 'Analyzing package manifest...' : 'Audit Manifest'}
                </button>
              </div>

              <div className="analysis-results-side glass-panel" data-testid="analyzer-result">
                <h3>Vulnerability & Audit Report</h3>
                {analyzeResult ? (
                  <div className="audit-output-wrapper">
                    <div className="audit-metrics-row">
                      <div className="metric-pill">
                        Cycles: <span className={`status-text ${analyzeResult.cyclesDetected ? 'failed' : 'success'}`}>{analyzeResult.cyclesDetected ? 'Detected' : 'None'}</span>
                      </div>
                      <div className="metric-pill">
                        Vulnerabilities: <span className={`status-text ${analyzeResult.vulnerabilityCount > 0 ? 'failed' : 'success'}`}>{analyzeResult.vulnerabilityCount}</span>
                      </div>
                    </div>

                    <div className="dependency-list-table">
                      <div className="table-header">
                        <span>Dependency</span>
                        <span>Version</span>
                        <span>Status</span>
                      </div>
                      {analyzeResult.dependencies.map((dep: any, idx: number) => (
                        <div key={idx} className="table-row">
                          <span className="dep-name">{dep.name}</span>
                          <span className="dep-version">{dep.version}</span>
                          <span className={`dep-status status-${dep.status}`}>{dep.status}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                ) : (
                  <div className="terminal-empty-state">
                    <Settings size={32} />
                    <p>Load cargo descriptor file above and run analysis audit.</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
