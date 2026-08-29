import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { LanguageSwitcher } from './components/LanguageSwitcher';
import { EventListenerDashboard } from './components/EventListenerDashboard';
import { TransactionSimulator } from './components/TransactionSimulator';
import { GasCostEstimator } from './components/GasCostEstimator';
import { MultiChainDashboard } from './components/MultiChainDashboard';
import { ContractAbiExplorer } from './components/ContractAbiExplorer';
import { DeveloperOnboardingTutorial } from './components/DeveloperOnboardingTutorial';
import { WalletConnector } from './components/WalletConnector';
import { AbiFormGenerator } from './components/AbiFormGenerator';
import { ContractFlowchartVisualizer } from './components/ContractFlowchartVisualizer';
import { TransactionTimeTravelDebugger } from './components/TransactionTimeTravelDebugger';
import { PortalNav } from './components/PortalNav';
import type { PortalTab } from './components/PortalNav';
import { Terminal, ShieldAlert, Cpu, Globe, Zap, Settings, RefreshCw, BookOpen, Wallet, Activity, Layers, FileJson, Workflow, History } from 'lucide-react';
import './App.css';

type Tab =
  | 'tutorial'
  | 'events'
  | 'simulator'
  | 'metrics'
  | 'multichain'
  | 'abi'
  | 'abiform'
  | 'flowchart'
  | 'debugger'
  | 'compiler'
  | 'dependencies'
  | 'wallet';

/** Single source of truth for the portal's views, shared by both nav layouts. */
const PORTAL_TABS: PortalTab[] = [
  { id: 'tutorial', label: 'Tutorial', icon: <BookOpen size={15} /> },
  { id: 'events', label: 'Event Listener', icon: <Activity size={15} /> },
  { id: 'simulator', label: 'Tx Simulator', icon: <Layers size={15} /> },
  { id: 'debugger', label: 'Time Travel', icon: <History size={15} /> },
  { id: 'metrics', label: 'Gas Estimator', icon: <Zap size={15} /> },
  { id: 'multichain', label: 'Node Manager', icon: <Globe size={15} /> },
  { id: 'abi', label: 'ABI Explorer', icon: <Cpu size={15} /> },
  { id: 'abiform', label: 'ABI Forms', icon: <FileJson size={15} /> },
  { id: 'flowchart', label: 'State Machine', icon: <Workflow size={15} /> },
  { id: 'compiler', label: 'Compiler Service', icon: <Terminal size={15} /> },
  { id: 'dependencies', label: 'Dep Analyzer', icon: <ShieldAlert size={15} /> },
  { id: 'wallet', label: 'Wallet', icon: <Wallet size={15} /> },
];

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

  const { t } = useTranslation();

  return (
    <div className="app-container">
      <header className="app-header">
        <div className="header-brand">
          <h1>{t('app.title', 'Crucible')} Developer Portal</h1>
          <div className="header-badge">Soroban Toolchain</div>
          <LanguageSwitcher />
        </div>
        <PortalNav
          tabs={PORTAL_TABS}
          activeTab={activeTab}
          onSelect={(id) => setActiveTab(id as Tab)}
        />
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
        {activeTab === 'abiform' && <AbiFormGenerator />}
        {activeTab === 'flowchart' && <ContractFlowchartVisualizer />}
        {activeTab === 'debugger' && <TransactionTimeTravelDebugger />}
        
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
