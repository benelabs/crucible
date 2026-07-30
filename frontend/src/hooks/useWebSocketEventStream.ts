import { useCallback, useEffect, useRef, useState } from 'react';
import type { ContractEvent } from '../components/eventFeed';

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'reconnecting' | 'paused';

export interface SubscriptionConfig {
  contractId?: string;
  topics?: string[];
  autoReconnect?: boolean;
  reconnectIntervalMs?: number;
  maxReconnectAttempts?: number;
  webSocketUrl?: string;
}

export interface UseWebSocketEventStreamReturn {
  events: ContractEvent[];
  connectionStatus: ConnectionStatus;
  subscribedContracts: string[];
  error: string | null;
  subscribe: (contractId: string, topics?: string[]) => void;
  unsubscribe: (contractId: string) => void;
  connect: () => void;
  disconnect: () => void;
  pause: () => void;
  resume: () => void;
  clearEvents: () => void;
}

const DEFAULT_WS_URL = 'wss://soroban-rpc.mainnet.stellar.org/ws/events';

export const useWebSocketEventStream = (
  config: SubscriptionConfig = {}
): UseWebSocketEventStreamReturn => {
  const {
    autoReconnect = true,
    reconnectIntervalMs = 3000,
    maxReconnectAttempts = 5,
    webSocketUrl = DEFAULT_WS_URL,
  } = config;

  const [events, setEvents] = useState<ContractEvent[]>([]);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('disconnected');
  const [subscribedContracts, setSubscribedContracts] = useState<string[]>(
    config.contractId ? [config.contractId] : []
  );
  const [error, setError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef<number | null>(null);
  const isPausedRef = useRef(false);

  const clearEvents = useCallback(() => {
    setEvents([]);
  }, []);

  const subscribe = useCallback((contractId: string) => {
    setSubscribedContracts((prev) =>
      prev.includes(contractId) ? prev : [...prev, contractId]
    );

    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(
        JSON.stringify({
          action: 'subscribe',
          contractId,
        })
      );
    }
  }, []);

  const unsubscribe = useCallback((contractId: string) => {
    setSubscribedContracts((prev) => prev.filter((id) => id !== contractId));

    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      wsRef.current.send(
        JSON.stringify({
          action: 'unsubscribe',
          contractId,
        })
      );
    }
  }, []);

  const disconnect = useCallback(() => {
    if (reconnectTimerRef.current) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    setConnectionStatus('disconnected');
  }, []);

  const connect = useCallback(() => {
    if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
      return;
    }

    setConnectionStatus(reconnectAttemptsRef.current > 0 ? 'reconnecting' : 'connecting');
    setError(null);

    try {
      const ws = new WebSocket(webSocketUrl);
      wsRef.current = ws;

      ws.onopen = () => {
        setConnectionStatus('connected');
        reconnectAttemptsRef.current = 0;
        setError(null);

        // Send pending subscriptions
        subscribedContracts.forEach((contractId) => {
          ws.send(JSON.stringify({ action: 'subscribe', contractId }));
        });
      };

      ws.onmessage = (event) => {
        if (isPausedRef.current) return;

        try {
          const parsed = JSON.parse(event.data);
          if (parsed.type === 'event' && parsed.data) {
            const newEvent: ContractEvent = parsed.data;
            setEvents((prev) => [newEvent, ...prev].slice(0, 100));
          } else if (Array.isArray(parsed)) {
            setEvents((prev) => [...parsed, ...prev].slice(0, 100));
          }
        } catch {
          // If raw event object is received
        }
      };

      ws.onerror = () => {
        setError('WebSocket streaming error encountered');
      };

      ws.onclose = () => {
        wsRef.current = null;
        if (!isPausedRef.current && autoReconnect && reconnectAttemptsRef.current < maxReconnectAttempts) {
          setConnectionStatus('reconnecting');
          reconnectAttemptsRef.current += 1;
          const delay = reconnectIntervalMs * Math.min(reconnectAttemptsRef.current, 3);
          reconnectTimerRef.current = window.setTimeout(() => {
            connect();
          }, delay);
        } else {
          setConnectionStatus('disconnected');
        }
      };
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to establish WebSocket connection');
      setConnectionStatus('disconnected');
    }
  }, [webSocketUrl, autoReconnect, reconnectIntervalMs, maxReconnectAttempts, subscribedContracts]);

  const pause = useCallback(() => {
    isPausedRef.current = true;
    setConnectionStatus('paused');
  }, []);

  const resume = useCallback(() => {
    isPausedRef.current = false;
    if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) {
      connect();
    } else {
      setConnectionStatus('connected');
    }
  }, [connect]);

  useEffect(() => {
    connect();
    return () => {
      disconnect();
    };
  }, [connect, disconnect]);

  return {
    events,
    connectionStatus,
    subscribedContracts,
    error,
    subscribe,
    unsubscribe,
    connect,
    disconnect,
    pause,
    resume,
    clearEvents,
  };
};
