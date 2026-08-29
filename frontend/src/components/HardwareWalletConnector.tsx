import React, { useCallback, useMemo, useState } from 'react';
import {
  Usb,
  ShieldCheck,
  AlertTriangle,
  Loader2,
  CheckCircle2,
  LogOut,
} from 'lucide-react';

import './HardwareWalletConnector.css';
import {
  createLedgerAdapter,
  defaultDerivationPaths,
  describeError,
  isWebHidSupported,
  type ConnectionState,
  type DerivedAccount,
  type HardwareError,
  type HardwareWalletAdapter,
} from './hardwareWallet';

export interface HardwareWalletConnectorProps {
  /** Injectable for tests and for adding a second vendor. */
  adapterFactory?: () => HardwareWalletAdapter;
  /** Override support detection (tests, or a forced-unsupported banner). */
  supported?: boolean;
  onConnected?: (account: DerivedAccount) => void;
  onDisconnected?: () => void;
}

const STEPS: Array<{ state: ConnectionState; label: string }> = [
  { state: 'requesting-device', label: 'Select your device' },
  { state: 'connecting', label: 'Open the Stellar app' },
  { state: 'fetching-key', label: 'Confirm the address on-device' },
];

/**
 * Hardware wallet connection via WebHID (issue #891).
 *
 * The device work lives in `hardwareWallet.ts`; this component owns the state
 * machine the user sees — which step is in progress, which derivation path is
 * selected, and what to do when something goes wrong.
 */
export const HardwareWalletConnector: React.FC<HardwareWalletConnectorProps> = ({
  adapterFactory = createLedgerAdapter,
  supported,
  onConnected,
  onDisconnected,
}) => {
  const webHidAvailable = supported ?? isWebHidSupported();

  const [state, setState] = useState<ConnectionState>('idle');
  const [adapter, setAdapter] = useState<HardwareWalletAdapter | null>(null);
  const [account, setAccount] = useState<DerivedAccount | null>(null);
  const [error, setError] = useState<HardwareError | null>(null);
  const [pathIndex, setPathIndex] = useState(0);

  const paths = useMemo(() => defaultDerivationPaths(5), []);
  const selectedPath = paths[pathIndex];

  const handleConnect = useCallback(async () => {
    setError(null);
    const next = adapterFactory();

    try {
      // Each step is surfaced separately because they fail for different
      // reasons and need different instructions: no device selected, app not
      // open, address declined on-device.
      setState('requesting-device');
      setState('connecting');
      await next.connect();

      setState('fetching-key');
      const publicKey = await next.getPublicKey(selectedPath);

      setAdapter(next);
      const derived = { derivationPath: selectedPath, publicKey };
      setAccount(derived);
      setState('connected');
      onConnected?.(derived);
    } catch (caught) {
      // The transport is closed on failure: a half-open HID handle blocks the
      // next attempt, and the device then appears "already in use" for
      // reasons the user cannot see.
      await next.disconnect().catch(() => {});
      setError(describeError(caught));
      setState('error');
    }
  }, [adapterFactory, onConnected, selectedPath]);

  const handleDisconnect = useCallback(async () => {
    await adapter?.disconnect().catch(() => {});
    setAdapter(null);
    setAccount(null);
    setState('idle');
    setError(null);
    onDisconnected?.();
  }, [adapter, onDisconnected]);

  const busy =
    state === 'requesting-device' ||
    state === 'connecting' ||
    state === 'fetching-key' ||
    state === 'awaiting-confirmation';

  if (!webHidAvailable) {
    return (
      <div className="hw-wallet" data-testid="hw-wallet-unsupported">
        <div className="hw-wallet__banner hw-wallet__banner--warn">
          <AlertTriangle size={16} aria-hidden="true" />
          <div>
            <strong>WebHID is not available in this browser.</strong>
            <p>
              Hardware wallet signing needs a Chromium-based browser on a
              secure (HTTPS) origin.
            </p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="hw-wallet" data-testid="hw-wallet">
      <header className="hw-wallet__header">
        <div className="header-icon-wrapper">
          <Usb size={22} className="header-icon" aria-hidden="true" />
        </div>
        <div>
          <h2>Hardware Wallet</h2>
          <p>Sign deployments and governance transactions on-device</p>
        </div>
      </header>

      {state !== 'connected' && (
        <>
          <label className="hw-wallet__field">
            <span>Derivation path</span>
            <select
              data-testid="derivation-path"
              value={pathIndex}
              disabled={busy}
              onChange={(e) => setPathIndex(Number(e.target.value))}
            >
              {paths.map((path, index) => (
                <option key={path} value={index}>
                  {path} — Account {index}
                </option>
              ))}
            </select>
          </label>

          <ol className="hw-wallet__steps" data-testid="hw-wallet-steps">
            {STEPS.map((step) => {
              const active = state === step.state;
              const done =
                STEPS.findIndex((s) => s.state === state) >
                STEPS.findIndex((s) => s.state === step.state);
              return (
                <li
                  key={step.state}
                  className={`hw-step ${active ? 'is-active' : ''} ${done ? 'is-done' : ''}`}
                  data-testid={`step-${step.state}`}
                >
                  {done ? (
                    <CheckCircle2 size={14} aria-hidden="true" />
                  ) : active ? (
                    <Loader2 size={14} className="spin" aria-hidden="true" />
                  ) : (
                    <span className="hw-step__dot" aria-hidden="true" />
                  )}
                  {step.label}
                </li>
              );
            })}
          </ol>

          <button
            type="button"
            className="hw-wallet__connect"
            data-testid="connect-button"
            disabled={busy}
            onClick={handleConnect}
          >
            {busy ? 'Connecting…' : 'Connect device'}
          </button>
        </>
      )}

      {error && (
        <div
          className="hw-wallet__banner hw-wallet__banner--error"
          role="alert"
          data-testid="hw-wallet-error"
        >
          <AlertTriangle size={16} aria-hidden="true" />
          <div>
            {/* The remedy leads, because the raw status word is meaningless
                to anyone not reading the APDU spec. */}
            <strong>{error.remedy}</strong>
            <p data-testid="hw-wallet-error-detail">{error.message}</p>
          </div>
        </div>
      )}

      {state === 'connected' && account && (
        <div className="hw-wallet__connected" data-testid="hw-wallet-connected">
          <div className="hw-wallet__banner hw-wallet__banner--ok">
            <ShieldCheck size={16} aria-hidden="true" />
            <div>
              <strong>Device connected</strong>
              <p data-testid="hw-wallet-path">{account.derivationPath}</p>
            </div>
          </div>
          <code data-testid="hw-wallet-key">{account.publicKey}</code>
          <button
            type="button"
            className="hw-wallet__disconnect"
            data-testid="disconnect-button"
            onClick={handleDisconnect}
          >
            <LogOut size={14} aria-hidden="true" />
            Disconnect
          </button>
        </div>
      )}
    </div>
  );
};

export default HardwareWalletConnector;
