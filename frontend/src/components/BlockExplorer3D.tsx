import React, { useEffect, useRef, useState } from 'react';
import { Play, Pause, RotateCcw, ZoomIn, ZoomOut, Info, ShieldCheck, Cpu, Database } from 'lucide-react';
import './BlockExplorer3D.css';

export interface LedgerBlockNode {
  id: string;
  ledgerNumber: number;
  hash: string;
  txCount: number;
  timestamp: string;
  x: number;
  y: number;
  z: number;
  status: 'confirmed' | 'pending' | 'failed';
  contractCalls: {
    id: string;
    contractId: string;
    functionName: string;
    caller: string;
    gasUsed: number;
  }[];
}

const MOCK_NODES: LedgerBlockNode[] = [
  {
    id: 'block-1045231',
    ledgerNumber: 1045231,
    hash: '0x8f3a9e1d2c4b5a6f7e8d9c0b1a2f3e4d5c6b7a8f',
    txCount: 42,
    timestamp: '2026-07-24 09:20:15 UTC',
    x: -120,
    y: 40,
    z: -50,
    status: 'confirmed',
    contractCalls: [
      {
        id: 'call-1',
        contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
        functionName: 'transfer',
        caller: 'GDQP2KPQGKIHYJG5BD2T8JVR...',
        gasUsed: 14500,
      },
    ],
  },
  {
    id: 'block-1045232',
    ledgerNumber: 1045232,
    hash: '0x1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b',
    txCount: 88,
    timestamp: '2026-07-24 09:20:20 UTC',
    x: -40,
    y: -30,
    z: 20,
    status: 'confirmed',
    contractCalls: [
      {
        id: 'call-2',
        contractId: 'CBQX2CLT7JFPASGQYQ6B6HR5IE23DVKSWJEVFXT7Y7AKLZ4E5YGH71MD',
        functionName: 'swap_tokens',
        caller: 'GA38P5K9L2M...',
        gasUsed: 32000,
      },
      {
        id: 'call-3',
        contractId: 'CA8P3ZD4EV2DJW66EXQ7K5IE3T3O7WMLTXIXR5YBZOSNQMPF5EYN4ZNQ',
        functionName: 'vote_proposal',
        caller: 'GB72K8N9L...',
        gasUsed: 8900,
      },
    ],
  },
  {
    id: 'block-1045233',
    ledgerNumber: 1045233,
    hash: '0x9f8e7d6c5b4a3f2e1d0c9b8a7f6e5d4c3b2a1f0e',
    txCount: 65,
    timestamp: '2026-07-24 09:20:25 UTC',
    x: 50,
    y: 60,
    z: -30,
    status: 'confirmed',
    contractCalls: [
      {
        id: 'call-4',
        contractId: 'CC4RQ3KX37R4XTQGDN3Q6O5IPTVRUKZSDHFDMB4JYCNKUEK9JH6B20KV',
        functionName: 'deposit_escrow',
        caller: 'GC11AA22BB...',
        gasUsed: 21000,
      },
    ],
  },
  {
    id: 'block-1045234',
    ledgerNumber: 1045234,
    hash: '0x3c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b',
    txCount: 19,
    timestamp: '2026-07-24 09:20:30 UTC',
    x: 140,
    y: -20,
    z: 40,
    status: 'pending',
    contractCalls: [
      {
        id: 'call-5',
        contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
        functionName: 'mint',
        caller: 'GD99LL88MM...',
        gasUsed: 11200,
      },
    ],
  },
];

export const BlockExplorer3D: React.FC = () => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [selectedNode, setSelectedNode] = useState<LedgerBlockNode | null>(MOCK_NODES[1]);
  const [isRotating, setIsRotating] = useState(true);
  const [zoomLevel, setZoomLevel] = useState(1);
  const [rotationAngle, setRotationAngle] = useState(0);

  // Render 3D Canvas Projection
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let animationFrameId: number;

    const render = () => {
      ctx.clearRect(0, 0, canvas.width, canvas.height);

      const centerX = canvas.width / 2;
      const centerY = canvas.height / 2;

      // Draw grid lines in 3D perspective
      ctx.strokeStyle = 'rgba(75, 85, 99, 0.2)';
      ctx.lineWidth = 1;
      for (let i = -200; i <= 200; i += 40) {
        ctx.beginPath();
        ctx.moveTo(centerX + i * zoomLevel, centerY - 150 * zoomLevel);
        ctx.lineTo(centerX + i * zoomLevel, centerY + 150 * zoomLevel);
        ctx.stroke();
      }

      // Draw Connection Lines between Nodes
      ctx.strokeStyle = 'rgba(99, 102, 241, 0.4)';
      ctx.lineWidth = 2 * zoomLevel;
      ctx.beginPath();
      MOCK_NODES.forEach((node, idx) => {
        const rad = (rotationAngle * Math.PI) / 180;
        const rotX = node.x * Math.cos(rad) - node.z * Math.sin(rad);
        const screenX = centerX + rotX * zoomLevel;
        const screenY = centerY + node.y * zoomLevel;

        if (idx === 0) {
          ctx.moveTo(screenX, screenY);
        } else {
          ctx.lineTo(screenX, screenY);
        }
      });
      ctx.stroke();

      // Render 3D Nodes
      MOCK_NODES.forEach((node) => {
        const rad = (rotationAngle * Math.PI) / 180;
        const rotX = node.x * Math.cos(rad) - node.z * Math.sin(rad);
        const rotZ = node.x * Math.sin(rad) + node.z * Math.cos(rad);

        const scale = (rotZ + 200) / 200;
        const radius = Math.max(12, 18 * scale * zoomLevel);
        const screenX = centerX + rotX * zoomLevel;
        const screenY = centerY + node.y * zoomLevel;

        const isSelected = selectedNode?.id === node.id;

        // Glow effect
        if (isSelected) {
          ctx.beginPath();
          ctx.arc(screenX, screenY, radius + 8, 0, 2 * Math.PI);
          ctx.fillStyle = 'rgba(129, 140, 248, 0.3)';
          ctx.fill();
        }

        // Main sphere gradient
        const gradient = ctx.createRadialGradient(
          screenX - radius * 0.3,
          screenY - radius * 0.3,
          radius * 0.1,
          screenX,
          screenY,
          radius
        );

        if (node.status === 'confirmed') {
          gradient.addColorStop(0, '#818cf8');
          gradient.addColorStop(1, '#4f46e5');
        } else {
          gradient.addColorStop(0, '#fbbf24');
          gradient.addColorStop(1, '#d97706');
        }

        ctx.beginPath();
        ctx.arc(screenX, screenY, radius, 0, 2 * Math.PI);
        ctx.fillStyle = gradient;
        ctx.fill();
        ctx.strokeStyle = isSelected ? '#ffffff' : '#312e81';
        ctx.lineWidth = isSelected ? 3 : 1.5;
        ctx.stroke();

        // Node Label
        ctx.fillStyle = '#f3f4f6';
        ctx.font = '12px sans-serif';
        ctx.textAlign = 'center';
        ctx.fillText(`#${node.ledgerNumber}`, screenX, screenY + radius + 16);
      });

      if (isRotating) {
        setRotationAngle((prev) => (prev + 0.5) % 360);
      }

      animationFrameId = requestAnimationFrame(render);
    };

    render();

    return () => {
      cancelAnimationFrame(animationFrameId);
    };
  }, [isRotating, rotationAngle, selectedNode, zoomLevel]);

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const clickY = e.clientY - rect.top;

    const centerX = canvas.width / 2;
    const centerY = canvas.height / 2;

    MOCK_NODES.forEach((node) => {
      const rad = (rotationAngle * Math.PI) / 180;
      const rotX = node.x * Math.cos(rad) - node.z * Math.sin(rad);
      const screenX = centerX + rotX * zoomLevel;
      const screenY = centerY + node.y * zoomLevel;

      const dist = Math.hypot(clickX - screenX, clickY - screenY);
      if (dist <= 25 * zoomLevel) {
        setSelectedNode(node);
      }
    });
  };

  return (
    <div className="webgl-explorer-container" data-testid="block-explorer-3d">
      <div className="webgl-header">
        <div className="webgl-title">
          <Database className="title-icon" />
          <h2>3D WebGL Ledger Explorer</h2>
        </div>
        <div className="webgl-controls">
          <button
            className="control-btn"
            onClick={() => setIsRotating(!isRotating)}
            title={isRotating ? 'Pause Rotation' : 'Auto Rotate'}
            data-testid="toggle-rotation-btn"
          >
            {isRotating ? <Pause size={16} /> : <Play size={16} />}
          </button>
          <button
            className="control-btn"
            onClick={() => setZoomLevel((prev) => Math.min(prev + 0.2, 2))}
            title="Zoom In"
            data-testid="zoom-in-btn"
          >
            <ZoomIn size={16} />
          </button>
          <button
            className="control-btn"
            onClick={() => setZoomLevel((prev) => Math.max(prev - 0.2, 0.6))}
            title="Zoom Out"
            data-testid="zoom-out-btn"
          >
            <ZoomOut size={16} />
          </button>
          <button
            className="control-btn"
            onClick={() => {
              setZoomLevel(1);
              setRotationAngle(0);
            }}
            title="Reset Camera"
            data-testid="reset-camera-btn"
          >
            <RotateCcw size={16} />
          </button>
        </div>
      </div>

      <div className="webgl-viewport">
        <canvas
          ref={canvasRef}
          width={700}
          height={380}
          className="webgl-canvas"
          onClick={handleCanvasClick}
          data-testid="webgl-canvas"
        />

        {selectedNode && (
          <div className="node-inspector-drawer" data-testid="node-inspector">
            <div className="drawer-header">
              <Cpu size={18} className="drawer-icon" />
              <h3>Ledger Block #{selectedNode.ledgerNumber}</h3>
              <span className={`status-badge status-${selectedNode.status}`}>
                {selectedNode.status}
              </span>
            </div>
            <div className="drawer-body">
              <div className="detail-row">
                <span className="label">Block Hash:</span>
                <span className="value hash">{selectedNode.hash}</span>
              </div>
              <div className="detail-row">
                <span className="label">Timestamp:</span>
                <span className="value">{selectedNode.timestamp}</span>
              </div>
              <div className="detail-row">
                <span className="label">Transactions:</span>
                <span className="value">{selectedNode.txCount} txs</span>
              </div>

              <h4 className="calls-heading">
                <ShieldCheck size={14} /> Smart Contract Invocations
              </h4>
              <div className="contract-calls-list">
                {selectedNode.contractCalls.map((call) => (
                  <div key={call.id} className="call-card">
                    <div className="call-header">
                      <span className="function-name">{call.functionName}()</span>
                      <span className="gas-tag">{call.gasUsed} gas</span>
                    </div>
                    <div className="call-details">
                      <div>Contract: <span className="mono">{call.contractId}</span></div>
                      <div>Caller: <span className="mono">{call.caller}</span></div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
