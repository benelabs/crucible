import React, { useState, useCallback } from 'react';
import { Wallet, LogOut, Copy, CheckCircle2, AlertTriangle, RefreshCw, ExternalLink } from 'lucide-react';
import './WalletConnector.css';

type WalletType = 'freighter' | 'albedo' | 'xbull';
type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';
type Network = 'mainnet' | 'testnet' | 'futurenet';

interface WalletInfo {
  type: WalletType;
  label: string;
  description: string;
  url: string;
}

interface ConnectedWallet {
  type: WalletType;
  publicKey: string;
  network: Network;
  balance: string;
}

const WALLETS: WalletInfo[] = [
  { type: 'freighter', label: 'Freighter', description: 'Browser extension wallet for Stellar', url: 'https://freighter.app' },
  { type: 'albedo', label: 'Albedo',    description: 'Web-based intent authorization service', url: 'https://albedo.link' },
  { type: 'xbull',   label: 'xBull',    description: 'Feature-rich mobile & desktop wallet',   url: 'https://xbull.app' },
];

const NETWORK_LABELS: Record<Network, string> = {
  mainnet:   'Mainnet',
  testnet:   'Testnet',
  futurenet: 'Futurenet',
};

/** Simulate wallet connection (replace with real SDK calls in production). */
async function mockConnect(walletType: WalletType, network: Network): Promise<ConnectedWallet> {
  await new Promise(r => setTimeout(r, 900));
  // In a real integration: use @stellar/freighter-api, albedo-link, etc.
  const mockKey = 'G' + Array.from({ length: 55 }, () => 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'[Math.floor(Math.random() * 32)]).join('');
  return { type: walletType, publicKey: mockKey, network, balance: (Math.random() * 1000).toFixed(2) };
}

export const WalletConnector: React.FC = () => {
  const [status, setStatus]       = useState<ConnectionStatus>('disconnected');
  const [wallet, setWallet]       = useState<ConnectedWallet | null>(null);
  const [network, setNetwork]     = useState<Network>('testnet');
  const [error, setError]         = useState<string | null>(null);
  const [copied, setCopied]       = useState(false);

  const handleConnect = useCallback(async (walletType: WalletType) => {
    setStatus('connecting');
    setError(null);
    try {
      const connected = await mockConnect(walletType, network);
      setWallet(connected);
      setStatus('connected');
    } catch (e: any) {
      setError(e.message ?? 'Connection failed');
      setStatus('error');
    }
  }, [network]);

  const handleDisconnect = useCallback(() => {
    setWallet(null);
    setStatus('disconnected');
    setError(null);
  }, []);

  const handleCopy = useCallback(async () => {
    if (!wallet) return;
    await navigator.clipboard.writeText(wallet.publicKey);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }, [wallet]);

  return (
    <div className="wallet-connector-container">
      <div className="wallet-header">
        <div className="header-icon-wrapper">
          <Wallet className="header-icon" />
        </div>
        <div>
          <h2>Wallet Connector</h2>
          <p>Connect a Stellar wallet to sign transactions and interact with deployed contracts</p>
        </div>
      </div>

      <div className="wallet-content">
        {/* Network selector — always visible */}
        <div className="network-selector glass-panel">
          <h3>Target Network</h3>
          <div className="network-tabs" role="group" aria-label="Select network">
            {(Object.keys(NETWORK_LABELS) as Network[]).map(n => (
              <button
                key={n}
                className={`network-tab ${network === n ? 'active' : ''}`}
                onClick={() => setNetwork(n)}
                disabled={status === 'connecting' || status === 'connected'}
                data-testid={`network-tab-${n}`}
              >
                {NETWORK_LABELS[n]}
              </button>
            ))}
          </div>
        </div>

        {/* Wallet list — shown when disconnected / error */}
        {(status === 'disconnected' || status === 'error') && (
          <div className="wallet-list glass-panel" data-testid="wallet-list">
            <h3>Choose a Wallet</h3>

            {error && (
              <div className="error-banner" role="alert" data-testid="error-banner">
                <AlertTriangle size={15} />
                <span>{error}</span>
              </div>
            )}

            <div className="wallet-options">
              {WALLETS.map(w => (
                <button
                  key={w.type}
                  className="wallet-option-card"
                  onClick={() => handleConnect(w.type)}
                  data-testid={`connect-${w.type}`}
                >
                  <div className="wallet-option-info">
                    <span className="wallet-option-label">{w.label}</span>
                    <span className="wallet-option-desc">{w.description}</span>
                  </div>
                  <a
                    href={w.url}
                    target="_blank"
                    rel="noreferrer"
                    className="wallet-ext-link"
                    onClick={e => e.stopPropagation()}
                    aria-label={`Open ${w.label} website`}
                  >
                    <ExternalLink size={13} />
                  </a>
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Connecting spinner */}
        {status === 'connecting' && (
          <div className="connecting-panel glass-panel" data-testid="connecting-panel">
            <RefreshCw size={28} className="spin" />
            <p>Connecting to wallet…</p>
          </div>
        )}

        {/* Connected state */}
        {status === 'connected' && wallet && (
          <div className="connected-panel glass-panel" data-testid="connected-panel">
            <div className="connected-header">
              <CheckCircle2 size={20} className="connected-icon" />
              <span className="connected-label">Connected via <strong>{WALLETS.find(w => w.type === wallet.type)?.label}</strong></span>
              <button
                className="disconnect-btn"
                onClick={handleDisconnect}
                data-testid="disconnect-button"
                aria-label="Disconnect wallet"
              >
                <LogOut size={14} />
                Disconnect
              </button>
            </div>

            <div className="wallet-details-grid">
              <div className="wallet-detail-card">
                <span className="detail-label">Network</span>
                <span className="detail-value" data-testid="connected-network">{NETWORK_LABELS[wallet.network]}</span>
              </div>
              <div className="wallet-detail-card">
                <span className="detail-label">XLM Balance</span>
                <span className="detail-value" data-testid="connected-balance">{wallet.balance} XLM</span>
              </div>
            </div>

            <div className="pubkey-row">
              <span className="detail-label">Public Key</span>
              <div className="pubkey-value-row">
                <code className="pubkey-text" data-testid="connected-pubkey">{wallet.publicKey}</code>
                <button
                  className="copy-btn"
                  onClick={handleCopy}
                  data-testid="copy-pubkey"
                  aria-label="Copy public key"
                >
                  {copied ? <CheckCircle2 size={14} /> : <Copy size={14} />}
                  {copied ? 'Copied!' : 'Copy'}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
