import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useWebSocketEventStream } from './useWebSocketEventStream';

class MockWebSocket {
  url: string;
  readyState = WebSocket.CONNECTING;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  sentMessages: string[] = [];

  constructor(url: string) {
    this.url = url;
    setTimeout(() => {
      this.readyState = WebSocket.OPEN;
      if (this.onopen) this.onopen();
    }, 10);
  }

  send(data: string) {
    this.sentMessages.push(data);
  }

  close() {
    this.readyState = WebSocket.CLOSED;
    if (this.onclose) this.onclose();
  }
}

describe('useWebSocketEventStream', () => {
  const originalWebSocket = global.WebSocket;

  beforeEach(() => {
    vi.stubGlobal('WebSocket', MockWebSocket);
  });

  afterEach(() => {
    vi.stubGlobal('WebSocket', originalWebSocket);
    vi.restoreAllMocks();
  });

  it('initializes with default state and connects', async () => {
    const { result } = renderHook(() =>
      useWebSocketEventStream({ autoReconnect: false })
    );

    expect(result.current.events).toEqual([]);
    expect(result.current.connectionStatus).toBe('connecting');

    // Wait for mock WebSocket open
    await vi.waitFor(() => {
      expect(result.current.connectionStatus).toBe('connected');
    });
  });

  it('allows subscribing and unsubscribing from contract IDs', () => {
    const { result } = renderHook(() =>
      useWebSocketEventStream({ autoReconnect: false })
    );

    act(() => {
      result.current.subscribe('CONTRACT_ABC');
    });

    expect(result.current.subscribedContracts).toContain('CONTRACT_ABC');

    act(() => {
      result.current.unsubscribe('CONTRACT_ABC');
    });

    expect(result.current.subscribedContracts).not.toContain('CONTRACT_ABC');
  });

  it('handles pause, resume, and clearEvents', () => {
    const { result } = renderHook(() =>
      useWebSocketEventStream({ autoReconnect: false })
    );

    act(() => {
      result.current.pause();
    });
    expect(result.current.connectionStatus).toBe('paused');

    act(() => {
      result.current.clearEvents();
    });
    expect(result.current.events).toEqual([]);
  });
});
