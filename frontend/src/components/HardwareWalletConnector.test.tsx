import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { HardwareWalletConnector } from './HardwareWalletConnector';
import {
  describeError,
  defaultDerivationPaths,
  isWebHidSupported,
  stellarDerivationPath,
  type HardwareWalletAdapter,
} from './hardwareWallet';

const KEY = 'GBA2KVYQZQVBJZ5S7HXJ4E3KPWXQ7ZMTZ7HN5XCPUZ2A4LMKJ5GQ6XW2';

/** Adapter stand-in — no device, no WebHID, fully scripted. */
function mockAdapter(
  over: Partial<HardwareWalletAdapter> = {},
): HardwareWalletAdapter & { disconnect: ReturnType<typeof vi.fn> } {
  const disconnect = vi.fn().mockResolvedValue(undefined);
  return {
    vendor: 'ledger',
    connect: vi.fn().mockResolvedValue(undefined),
    getPublicKey: vi.fn().mockResolvedValue(KEY),
    signTransaction: vi.fn().mockResolvedValue(new Uint8Array([1, 2, 3])),
    disconnect,
    ...over,
  } as HardwareWalletAdapter & { disconnect: ReturnType<typeof vi.fn> };
}

describe('stellarDerivationPath', () => {
  it('builds the BIP-44 Stellar path', () => {
    expect(stellarDerivationPath(0)).toBe("44'/148'/0'");
    expect(stellarDerivationPath(3)).toBe("44'/148'/3'");
  });

  it('rejects a negative or fractional account index', () => {
    expect(() => stellarDerivationPath(-1)).toThrow(RangeError);
    expect(() => stellarDerivationPath(1.5)).toThrow(RangeError);
  });

  it('offers a distinct path per account in the picker', () => {
    const paths = defaultDerivationPaths(5);
    expect(paths).toHaveLength(5);
    expect(new Set(paths).size).toBe(5);
  });
});

describe('isWebHidSupported', () => {
  it('is false without a navigator', () => {
    expect(isWebHidSupported(undefined)).toBe(false);
  });

  it('is false when the navigator has no hid', () => {
    expect(isWebHidSupported({} as Navigator)).toBe(false);
  });

  it('is true when hid is present', () => {
    expect(isWebHidSupported({ hid: {} } as unknown as Navigator)).toBe(true);
  });
});

describe('describeError', () => {
  it('turns 0x6511 into "open the Stellar app"', () => {
    const described = describeError(new Error('Ledger device: 0x6511'));
    expect(described.code).toBe('app-not-open');
    expect(described.remedy).toContain('Stellar app');
  });

  it('recognises a locked device', () => {
    expect(describeError(new Error('0x6982')).code).toBe('device-locked');
  });

  it('recognises an on-device rejection', () => {
    expect(describeError(new Error('Condition of use not satisfied 0x6985')).code)
      .toBe('user-rejected');
  });

  it('recognises no device selected', () => {
    expect(describeError(new Error('No device selected.')).code).toBe(
      'no-device',
    );
  });

  it('always returns something actionable for an unknown failure', () => {
    const described = describeError('something odd');
    expect(described.code).toBe('unknown');
    expect(described.remedy.length).toBeGreaterThan(0);
  });
});

describe('HardwareWalletConnector', () => {
  it('explains itself when WebHID is unavailable instead of offering a dead button', () => {
    render(<HardwareWalletConnector supported={false} />);

    expect(screen.getByTestId('hw-wallet-unsupported')).toBeInTheDocument();
    expect(screen.queryByTestId('connect-button')).not.toBeInTheDocument();
  });

  it('walks from idle to connected and reports the key', async () => {
    const onConnected = vi.fn();
    const adapter = mockAdapter();

    render(
      <HardwareWalletConnector
        supported
        adapterFactory={() => adapter}
        onConnected={onConnected}
      />,
    );

    fireEvent.click(screen.getByTestId('connect-button'));

    await waitFor(() =>
      expect(screen.getByTestId('hw-wallet-connected')).toBeInTheDocument(),
    );
    expect(screen.getByTestId('hw-wallet-key')).toHaveTextContent(KEY);
    expect(onConnected).toHaveBeenCalledWith({
      derivationPath: "44'/148'/0'",
      publicKey: KEY,
    });
  });

  it('derives the key for the selected path, not always account 0', async () => {
    const adapter = mockAdapter();

    render(
      <HardwareWalletConnector supported adapterFactory={() => adapter} />,
    );

    fireEvent.change(screen.getByTestId('derivation-path'), {
      target: { value: '2' },
    });
    fireEvent.click(screen.getByTestId('connect-button'));

    await waitFor(() =>
      expect(adapter.getPublicKey).toHaveBeenCalledWith("44'/148'/2'"),
    );
  });

  it('leads with the remedy when the device reports an error', async () => {
    const adapter = mockAdapter({
      connect: vi.fn().mockRejectedValue(new Error('Ledger device: 0x6511')),
    });

    render(
      <HardwareWalletConnector supported adapterFactory={() => adapter} />,
    );
    fireEvent.click(screen.getByTestId('connect-button'));

    const banner = await screen.findByTestId('hw-wallet-error');
    expect(banner).toHaveTextContent('Open the Stellar app');
    // The raw status word is kept, but subordinate to the instruction.
    expect(screen.getByTestId('hw-wallet-error-detail')).toHaveTextContent(
      '0x6511',
    );
  });

  it('closes the transport when connection fails', async () => {
    // A half-open HID handle blocks the next attempt, and the device then
    // looks "already in use" for reasons the user cannot see.
    const adapter = mockAdapter({
      getPublicKey: vi.fn().mockRejectedValue(new Error('0x6985')),
    });

    render(
      <HardwareWalletConnector supported adapterFactory={() => adapter} />,
    );
    fireEvent.click(screen.getByTestId('connect-button'));

    await waitFor(() => expect(adapter.disconnect).toHaveBeenCalled());
    expect(screen.queryByTestId('hw-wallet-connected')).not.toBeInTheDocument();
  });

  it('stays connectable after a failure', async () => {
    const connect = vi
      .fn()
      .mockRejectedValueOnce(new Error('0x6511'))
      .mockResolvedValue(undefined);
    const adapter = mockAdapter({ connect });

    render(
      <HardwareWalletConnector supported adapterFactory={() => adapter} />,
    );

    fireEvent.click(screen.getByTestId('connect-button'));
    await screen.findByTestId('hw-wallet-error');

    fireEvent.click(screen.getByTestId('connect-button'));
    await waitFor(() =>
      expect(screen.getByTestId('hw-wallet-connected')).toBeInTheDocument(),
    );
    expect(screen.queryByTestId('hw-wallet-error')).not.toBeInTheDocument();
  });

  it('disconnects and returns to idle', async () => {
    const onDisconnected = vi.fn();
    const adapter = mockAdapter();

    render(
      <HardwareWalletConnector
        supported
        adapterFactory={() => adapter}
        onDisconnected={onDisconnected}
      />,
    );

    fireEvent.click(screen.getByTestId('connect-button'));
    await screen.findByTestId('hw-wallet-connected');

    fireEvent.click(screen.getByTestId('disconnect-button'));

    await waitFor(() =>
      expect(screen.getByTestId('connect-button')).toBeInTheDocument(),
    );
    expect(adapter.disconnect).toHaveBeenCalled();
    expect(onDisconnected).toHaveBeenCalled();
  });

  it('disables the connect button while a connection is in flight', async () => {
    let release: () => void = () => {};
    const adapter = mockAdapter({
      connect: vi.fn(
        () =>
          new Promise<void>((resolve) => {
            release = resolve;
          }),
      ),
    });

    render(
      <HardwareWalletConnector supported adapterFactory={() => adapter} />,
    );

    fireEvent.click(screen.getByTestId('connect-button'));
    await waitFor(() =>
      expect(screen.getByTestId('connect-button')).toBeDisabled(),
    );

    release();
    await screen.findByTestId('hw-wallet-connected');
  });
});
