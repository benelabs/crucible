export type EventSeverity = 'info' | 'success' | 'warning' | 'critical';

export type ListenerStatus = 'connected' | 'paused' | 'disconnected';

export type StellarEventType = 'contract' | 'system';

export type ContractEvent = {
  id: string;
  pagingToken: string;
  type: StellarEventType;
  contractId: string;
  ledger: number;
  ledgerClosedAt: string;
  topic: string[];
  value: string;
  txHash: string;
  displayName: string;
  topicLabel: string;
  valuePreview: string;
  severity: EventSeverity;
};

export type EventFilter = {
  severity: 'all' | EventSeverity;
  query: string;
};

const eventTemplates = [
  {
    contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
    displayName: 'transfer',
    severity: 'success' as EventSeverity,
    topic: ['AAAADwAAAAh0cmFuc2Zlcg==', '*', '*', '**'],
    topicLabel: 'token.transfer',
    value: 'AAAACgAAAAEAAAAPAAAABWFtb3VudAAAAAYAAAAAAADA0A==',
    valuePreview: '12,500 USDC moved from treasury to rewards vault',
    txHash: '32f7e5c3afd281fcaa99c0e990adf62f33e3bb341b1641a5c8b0b4a4dc55c487',
  },
  {
    contractId: 'CBQX2CLT7JFPASGQYQ6B6HR5IE23DVKSWJEVFXT7Y7AKLZ4E5YGH71MD',
    displayName: 'approval',
    severity: 'info' as EventSeverity,
    topic: ['AAAADwAAAAhhcHByb3ZlZA==', '*', '*'],
    topicLabel: 'allowance.approved',
    value: 'AAAADwAAADJMaXF1aWRpdHkgcm91dGVyIGFsbG93YW5jZSB1cGRhdGVk',
    valuePreview: 'Liquidity router allowance updated for market maker account',
    txHash: '8a6e9b06e1127f1f7d88e7ce35d37cc4589f4f9f55cdd42485e665d7ce92a114',
  },
  {
    contractId: 'CC4RQ3KX37R4XTQGDN3Q6O5IPTVRUKZSDHFDMB4JYCNKUEK9JH6B20KV',
    displayName: 'escrow_release',
    severity: 'warning' as EventSeverity,
    topic: ['AAAADwAAAAZlc2Nyb3c=', 'AAAADwAAAAdyZWxlYXNl'],
    topicLabel: 'escrow.release',
    value: 'AAAADwAAACpFc2Nyb3cgcmVsZWFzZSBvYnNlcnZlZCBuZWFyIHRpbWVvdXQ=',
    valuePreview: 'Escrow release observed near timeout threshold',
    txHash: 'a4c8d6b970d5e0c629f9f1a6e45bbd63d08a4b5c87dfdcf7343a36f5f9857e11',
  },
  {
    contractId: 'CA8P3ZD4EV2DJW66EXQ7K5IE3T3O7WMLTXIXR5YBZOSNQMPF5EYN4ZNQ',
    displayName: 'admin_call',
    severity: 'critical' as EventSeverity,
    topic: ['AAAADwAAAAphZG1pbl9jYWxs', '*'],
    topicLabel: 'governance.admin',
    value: 'AAAADwAAADBBZG1pbiByb2xlIGF0dGVtcHRlZCBwcml2aWxlZ2VkIHVwZGF0ZQ==',
    valuePreview: 'Admin role attempted privileged configuration update',
    txHash: 'f9717cfa77cb79d248772485404da891194dc776af13a2c6cbe1a75ea33d5421',
  },
  {
    contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
    displayName: 'mint',
    severity: 'success' as EventSeverity,
    topic: ['AAAADwAAAARtaW50', '*'],
    topicLabel: 'asset.mint',
    value: 'AAAADwAAACxBdXRob3JpemVkIG1pbnQgY29tcGxldGVkIGZvciBlbWlzc2lvbnM=',
    valuePreview: 'Authorized mint completed for emissions distributor',
    txHash: 'c564e65f3cae889b428ab098cc12f8b836ccdbd483df2e31f0f93846cdb9f7af',
  },
  {
    contractId: 'CBQX2CLT7JFPASGQYQ6B6HR5IE23DVKSWJEVFXT7Y7AKLZ4E5YGH71MD',
    displayName: 'fee_update',
    severity: 'info' as EventSeverity,
    topic: ['AAAADwAAAANmZWU=', 'AAAADwAAAAd1cGRhdGVk'],
    topicLabel: 'protocol.fee',
    value: 'AAAADwAAMFByb3RvY29sIGZlZSBzY2hlZHVsZSB1cGRhdGVkIGZyb20gY2hlY2twb2ludA==',
    valuePreview: 'Protocol fee schedule updated from listener checkpoint',
    txHash: '86e44f4d7340e48f84b671f7332ff7b1e4a38d7b3d9a29ab8c4d1373d5584a6c',
  },
];

const initialTimestamps = [
  '2026-05-31T13:40:24.000Z',
  '2026-05-31T13:39:58.000Z',
  '2026-05-31T13:39:31.000Z',
  '2026-05-31T13:39:05.000Z',
  '2026-05-31T13:38:42.000Z',
];

const initialEventIds = [
  '0000859036408881152-0000000001',
  '0000859036408881151-0000000002',
  '0000859036408881150-0000000003',
  '0000859036408881149-0000000004',
  '0000859036408881148-0000000005',
];

export const createInitialEvents = (): ContractEvent[] =>
  eventTemplates.slice(0, 5).map((template, index) => ({
    ...template,
    id: initialEventIds[index],
    pagingToken: initialEventIds[index],
    type: 'contract',
    ledger: 12_940_250 - index,
    ledgerClosedAt: initialTimestamps[index],
  }));

export const createNextEvent = (sequence: number, baseLedger: number): ContractEvent => {
  const template = eventTemplates[sequence % eventTemplates.length];
  const eventId = `000085903640888${String(2000 + sequence).padStart(4, '0')}-0000000001`;
  return {
    ...template,
    id: eventId,
    pagingToken: eventId,
    type: 'contract',
    ledger: baseLedger + sequence,
    ledgerClosedAt: new Date(Date.now()).toISOString(),
  };
};

export const filterEvents = (events: ContractEvent[], filter: EventFilter): ContractEvent[] => {
  const query = filter.query.trim().toLowerCase();

  return events.filter((event) => {
    const severityMatches = filter.severity === 'all' || event.severity === filter.severity;
    if (!severityMatches) {
      return false;
    }

    if (!query) {
      return true;
    }

    return [
      event.contractId,
      event.displayName,
      event.topicLabel,
      event.topic.join(' '),
      event.value,
      event.valuePreview,
      event.txHash,
      event.type,
      String(event.ledger),
    ].some((value) => value.toLowerCase().includes(query));
  });
};

export const formatEventTime = (timestamp: string): string =>
  new Intl.DateTimeFormat('en', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(new Date(timestamp));
