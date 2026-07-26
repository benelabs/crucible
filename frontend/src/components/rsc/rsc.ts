/**
 * React Server Components (RSC) Utilities and Abstractions (Issue #691)
 *
 * Facilitates offloading heavy dashboard data processing to server-side execution,
 * reducing client bundle size and improving Time-To-Interactive (TTI).
 */

export interface DashboardServerData {
  contractsActive: number;
  totalTransactions: number;
  systemHealth: string;
  responseLatencyMs: number;
  timestamp: string;
}

export interface ServerComponentProps<T> {
  serverDataFetcher: () => Promise<T>;
  children: (data: T) => React.ReactNode;
  fallback?: React.ReactNode;
}

/**
 * Server-side data fetcher for Dashboard metrics
 */
export async function fetchDashboardDataServer(): Promise<DashboardServerData> {
  // Simulates server-side data retrieval directly from database/cache
  return {
    contractsActive: 142,
    totalTransactions: 158900,
    systemHealth: 'OPERATIONAL',
    responseLatencyMs: 14.2,
    timestamp: new Date().toISOString(),
  };
}

/**
 * Server-side data fetcher for Event Listener metrics
 */
export async function fetchEventListenerDataServer() {
  return {
    eventsProcessed: 94200,
    activeSubscriptions: 18,
    lastEventBlock: 4892011,
  };
}
