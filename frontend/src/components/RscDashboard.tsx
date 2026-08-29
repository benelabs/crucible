import React, { useEffect, useState } from 'react';
import { fetchDashboardDataServer, DashboardServerData } from './rsc/rsc';
import './MultiChainDashboard.css';

export const RscDashboard: React.FC = () => {
  const [data, setData] = useState<DashboardServerData | null>(null);
  const [loading, setLoading] = useState<boolean>(true);

  useEffect(() => {
    let isMounted = true;
    fetchDashboardDataServer().then((serverData) => {
      if (isMounted) {
        setData(serverData);
        setLoading(false);
      }
    });
    return () => {
      isMounted = false;
    };
  }, []);

  if (loading || !data) {
    return (
      <div className="rsc-dashboard-loading" data-testid="rsc-loading">
        <p>Loading Server-Rendered Dashboard Component (RSC)...</p>
      </div>
    );
  }

  return (
    <div className="rsc-dashboard-container" data-testid="rsc-dashboard">
      <header className="rsc-dashboard-header">
        <h2>Crucible RSC Dashboard (Server Components)</h2>
        <span className="rsc-badge">RSC Enabled</span>
      </header>

      <div className="rsc-grid">
        <div className="rsc-card" data-testid="active-contracts-card">
          <h3>Active Contracts</h3>
          <p className="rsc-value">{data.contractsActive}</p>
        </div>

        <div className="rsc-card" data-testid="transactions-card">
          <h3>Total Transactions</h3>
          <p className="rsc-value">{data.totalTransactions.toLocaleString()}</p>
        </div>

        <div className="rsc-card" data-testid="health-card">
          <h3>System Status</h3>
          <p className="rsc-value status-ok">{data.systemHealth}</p>
        </div>

        <div className="rsc-card" data-testid="latency-card">
          <h3>Avg Response Latency</h3>
          <p className="rsc-value">{data.responseLatencyMs} ms</p>
        </div>
      </div>
    </div>
  );
};

export default RscDashboard;
