import { useState, useMemo } from 'react';

// Domain Types
export type ContractType = 'token' | 'nft' | 'defi' | 'escrow' | 'vesting';

export interface ContractTemplate {
  id: ContractType;
  name: string;
  contractId: string;
  functions: ContractFunction[];
}

export interface ContractFunction {
  name: string;
  description: string;
  arguments: FunctionArgument[];
  requiresAuth: string[]; // addresses/names required to sign
}

export interface FunctionArgument {
  name: string;
  type: 'string' | 'address' | 'i128' | 'u32';
  defaultValue: string;
  placeholder: string;
  description: string;
}

export interface AccountOption {
  name: string;
  address: string;
  description: string;
}

export interface SimConfig {
  ledgerSeq: number;
  ledgerTimestamp: number;
  protocolVersion: number;
  cpuLimit: number;
  ramLimit: number;
  feeRate: number; // stroops per operation
}

export interface SimEvent {
  contractId: string;
  topics: string[];
  topicLabel: string;
  valuePreview: string;
}

export interface SimulationResult {
  success: boolean;
  errorCode?: string;
  errorMessage?: string;
  cpuConsumed: number;
  ramConsumed: number;
  estimatedFee: number; // in stroops
  readKeys: string[];
  writeKeys: string[];
  emittedEvents: SimEvent[];
  returnValue: string;
  traceLogs: string[];
}

// Preset Mock Accounts
export const MOCK_ACCOUNTS: AccountOption[] = [
  { name: 'alice', address: 'GBA2K...ALICE...4XW2', description: 'Alice (User account)' },
  { name: 'bob', address: 'GBB3L...BOB...5YX3', description: 'Bob (User account)' },
  { name: 'admin', address: 'GBC4M...ADMIN...6ZY4', description: 'Admin (Contract administrator)' },
  { name: 'arbiter', address: 'GBD5N...ARBITER...7XZ5', description: 'Arbiter (Escrow multi-sig referee)' },
];

// Preset Contract Templates and Functions
export const CONTRACT_TEMPLATES: ContractTemplate[] = [
  {
    id: 'token',
    name: 'Token Contract',
    contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
    functions: [
      {
        name: 'transfer',
        description: 'Transfer tokens from one account to another',
        requiresAuth: ['from'],
        arguments: [
          { name: 'from', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Source address' },
          { name: 'to', type: 'address', defaultValue: 'bob', placeholder: 'GB...', description: 'Destination address' },
          { name: 'amount', type: 'i128', defaultValue: '1000', placeholder: 'e.g. 1000', description: 'Amount to transfer in stroops' },
        ],
      },
      {
        name: 'mint',
        description: 'Mint new tokens into an account (privileged)',
        requiresAuth: ['admin'],
        arguments: [
          { name: 'to', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Recipient address' },
          { name: 'amount', type: 'i128', defaultValue: '50000', placeholder: 'e.g. 50000', description: 'Amount to mint' },
        ],
      },
      {
        name: 'burn',
        description: 'Burn tokens from an account',
        requiresAuth: ['from'],
        arguments: [
          { name: 'from', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Account to burn tokens from' },
          { name: 'amount', type: 'i128', defaultValue: '500', placeholder: 'e.g. 500', description: 'Amount to burn' },
        ],
      },
      {
        name: 'approve',
        description: 'Approve a spender to withdraw up to a certain amount',
        requiresAuth: ['from'],
        arguments: [
          { name: 'from', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Token owner' },
          { name: 'spender', type: 'address', defaultValue: 'admin', placeholder: 'GB...', description: 'Approved spender' },
          { name: 'amount', type: 'i128', defaultValue: '10000', placeholder: 'e.g. 10000', description: 'Maximum allowance' },
        ],
      },
    ],
  },
  {
    id: 'nft',
    name: 'NFT (Non-Fungible Token)',
    contractId: 'CBMINT3NFT2Y3K4T67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCNFT',
    functions: [
      {
        name: 'mint_nft',
        description: 'Mint a new unique NFT (privileged)',
        requiresAuth: ['admin'],
        arguments: [
          { name: 'to', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'NFT owner' },
          { name: 'tokenId', type: 'u32', defaultValue: '1', placeholder: 'e.g. 1', description: 'Unique token index identifier' },
        ],
      },
      {
        name: 'transfer_nft',
        description: 'Transfer ownership of an NFT',
        requiresAuth: ['from'],
        arguments: [
          { name: 'from', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Current owner' },
          { name: 'to', type: 'address', defaultValue: 'bob', placeholder: 'GB...', description: 'New owner' },
          { name: 'tokenId', type: 'u32', defaultValue: '1', placeholder: 'e.g. 1', description: 'Token identifier' },
        ],
      },
    ],
  },
  {
    id: 'defi',
    name: 'DeFi Swapper',
    contractId: 'CBQX2CLT7JFPASGQYQ6B6HR5IE23DVKSWJEVFXT7Y7AKLZ4E5YGH71MD',
    functions: [
      {
        name: 'swap',
        description: 'Swap Token A for Token B on AMM pool',
        requiresAuth: ['from'],
        arguments: [
          { name: 'from', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Trader address' },
          { name: 'amountIn', type: 'i128', defaultValue: '5000', placeholder: 'e.g. 5000', description: 'Amount of Token A input' },
          { name: 'minAmountOut', type: 'i128', defaultValue: '4800', placeholder: 'e.g. 4800', description: 'Minimum acceptable Token B output' },
        ],
      },
      {
        name: 'add_liquidity',
        description: 'Provide dual-sided liquidity into a pool',
        requiresAuth: ['from'],
        arguments: [
          { name: 'from', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Liquidity provider' },
          { name: 'amountA', type: 'i128', defaultValue: '100000', placeholder: 'e.g. 100000', description: 'Amount of Token A' },
          { name: 'amountB', type: 'i128', defaultValue: '100000', placeholder: 'e.g. 100000', description: 'Amount of Token B' },
        ],
      },
    ],
  },
  {
    id: 'escrow',
    name: 'Escrow Vault',
    contractId: 'CC4RQ3KX37R4XTQGDN3Q6O5IPTVRUKZSDHFDMB4JYCNKUEK9JH6B20KV',
    functions: [
      {
        name: 'create_escrow',
        description: 'Create an escrow lockup funded by the buyer',
        requiresAuth: ['buyer'],
        arguments: [
          { name: 'buyer', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Funding buyer' },
          { name: 'seller', type: 'address', defaultValue: 'bob', placeholder: 'GB...', description: 'Target seller' },
          { name: 'arbiter', type: 'address', defaultValue: 'arbiter', placeholder: 'GD...', description: 'Dispute arbiter' },
          { name: 'amount', type: 'i128', defaultValue: '25000', placeholder: 'e.g. 25000', description: 'Escrow amount' },
          { name: 'lockPeriodDays', type: 'u32', defaultValue: '7', placeholder: 'e.g. 7', description: 'Vault timeout period in days' },
        ],
      },
      {
        name: 'claim_escrow',
        description: 'Claim escrow funds after arbiter authorization',
        requiresAuth: ['arbiter'],
        arguments: [
          { name: 'escrowId', type: 'u32', defaultValue: '101', placeholder: 'e.g. 101', description: 'Active escrow vault ID' },
          { name: 'recipient', type: 'address', defaultValue: 'bob', placeholder: 'GB...', description: 'Recipient address' },
        ],
      },
      {
        name: 'refund_escrow',
        description: 'Refund escrow funds back to buyer (only after lock timeout)',
        requiresAuth: ['buyer'],
        arguments: [
          { name: 'escrowId', type: 'u32', defaultValue: '101', placeholder: 'e.g. 101', description: 'Escrow vault ID' },
        ],
      },
    ],
  },
  {
    id: 'vesting',
    name: 'Token Vesting',
    contractId: 'CA8P3ZD4EV2DJW66EXQ7K5IE3T3O7WMLTXIXR5YBZOSNQMPF5EYN4ZNQ',
    functions: [
      {
        name: 'create_vesting',
        description: 'Schedule a vesting plan for a beneficiary (privileged)',
        requiresAuth: ['admin'],
        arguments: [
          { name: 'beneficiary', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Vesting beneficiary' },
          { name: 'amount', type: 'i128', defaultValue: '1000000', placeholder: 'e.g. 1000000', description: 'Total vestable amount' },
          { name: 'cliffDays', type: 'u32', defaultValue: '90', placeholder: 'e.g. 90', description: 'Cliff period in days' },
          { name: 'durationDays', type: 'u32', defaultValue: '360', placeholder: 'e.g. 360', description: 'Total schedule period in days' },
        ],
      },
      {
        name: 'claim_vested',
        description: 'Claim vested shares (enforces cliff constraints)',
        requiresAuth: ['beneficiary'],
        arguments: [
          { name: 'vestingId', type: 'u32', defaultValue: '12', placeholder: 'e.g. 12', description: 'Vesting schedule ID' },
          { name: 'beneficiary', type: 'address', defaultValue: 'alice', placeholder: 'GA...', description: 'Beneficiary claiming' },
        ],
      },
    ],
  },
];

export const useTransactionSimulator = () => {
  // Config state
  const [selectedContractId, setSelectedContractId] = useState<ContractType>('token');
  const [selectedFunctionName, setSelectedFunctionName] = useState<string>('transfer');
  
  // Dynamic arguments mapping
  const [args, setArgs] = useState<Record<string, string>>({
    from: 'alice',
    to: 'bob',
    amount: '1000',
  });

  // Auth Signatures Verified state
  const [authSignatures, setAuthSignatures] = useState<Record<string, boolean>>({
    alice: true,
    bob: false,
    admin: true,
    arbiter: true,
  });

  // Environment Settings
  const [envConfig, setEnvConfig] = useState<SimConfig>({
    ledgerSeq: 12940250,
    ledgerTimestamp: Math.floor(Date.now() / 1000),
    protocolVersion: 21,
    cpuLimit: 100000000, // 100M instructions Soroban limit
    ramLimit: 40960, // 40MB limit
    feeRate: 100, // 100 stroops base fee
  });

  // Simulator operation state
  const [isSimulating, setIsSimulating] = useState<boolean>(false);
  const [simulationResult, setSimulationResult] = useState<SimulationResult | null>(null);

  const selectedContract = useMemo(() => {
    return CONTRACT_TEMPLATES.find((c) => c.id === selectedContractId)!;
  }, [selectedContractId]);

  const selectedFunction = useMemo(() => {
    return selectedContract.functions.find((f) => f.name === selectedFunctionName) || selectedContract.functions[0];
  }, [selectedContract, selectedFunctionName]);

  // Adjust parameters when switching contract or function
  const handleSelectContract = (contractType: ContractType) => {
    const template = CONTRACT_TEMPLATES.find((c) => c.id === contractType)!;
    const initialFunc = template.functions[0];
    setSelectedContractId(contractType);
    setSelectedFunctionName(initialFunc.name);
    
    // Set default arguments
    const initialArgs: Record<string, string> = {};
    initialFunc.arguments.forEach((arg) => {
      initialArgs[arg.name] = arg.defaultValue;
    });
    setArgs(initialArgs);
  };

  const handleSelectFunction = (funcName: string) => {
    const func = selectedContract.functions.find((f) => f.name === funcName)!;
    setSelectedFunctionName(funcName);

    // Set default arguments
    const initialArgs: Record<string, string> = {};
    func.arguments.forEach((arg) => {
      initialArgs[arg.name] = arg.defaultValue;
    });
    setArgs(initialArgs);
  };

  const handleArgChange = (name: string, value: string) => {
    setArgs((prev) => ({
      ...prev,
      [name]: value,
    }));
  };

  const toggleAuthSignature = (accountName: string) => {
    setAuthSignatures((prev) => ({
      ...prev,
      [accountName]: !prev[accountName],
    }));
  };

  const handleEnvChange = <K extends keyof SimConfig>(key: K, value: SimConfig[K]) => {
    setEnvConfig((prev) => ({
      ...prev,
      [key]: value,
    }));
  };

  const resetConfig = () => {
    handleSelectContract('token');
    setAuthSignatures({
      alice: true,
      bob: false,
      admin: true,
      arbiter: true,
    });
    setEnvConfig({
      ledgerSeq: 12940250,
      ledgerTimestamp: Math.floor(Date.now() / 1000),
      protocolVersion: 21,
      cpuLimit: 100000000,
      ramLimit: 40960,
      feeRate: 100,
    });
    setSimulationResult(null);
  };

  // Run transaction simulator
  const runSimulation = () => {
    setIsSimulating(true);
    setSimulationResult(null);

    setTimeout(() => {
      // 1. Gather simulation context and arguments
      const currentContract = selectedContract;
      const currentFunction = selectedFunction;
      const currentArgs = { ...args };

      // 2. Perform validations & check auth signatures
      const requiredAuths = currentFunction.requiresAuth;
      const missingSignatures: string[] = [];

      requiredAuths.forEach((reqName) => {
        // Resolve target account name from arguments or settings
        const argValue = currentArgs[reqName];
        const targetAccount = MOCK_ACCOUNTS.find((acc) => acc.name === argValue || acc.name === reqName);
        if (targetAccount) {
          if (!authSignatures[targetAccount.name]) {
            missingSignatures.push(targetAccount.name);
          }
        } else {
          // Fallback if no matching arg name, check literal signature
          if (!authSignatures[reqName]) {
            missingSignatures.push(reqName);
          }
        }
      });

      // Initialize logs
      const trace: string[] = [
        `[soroban-vm] [info] Initializing Soroban Host VM Environment`,
        `[soroban-vm] [info] Protocol Version: ${envConfig.protocolVersion}`,
        `[soroban-vm] [info] Ledger Sequence: ${envConfig.ledgerSeq} | Timestamp: ${envConfig.ledgerTimestamp}`,
        `[soroban-vm] [info] Loading contract WASM bytecode for ID: ${currentContract.contractId}`,
        `[soroban-vm] [info] Parsing exports... Found function: "${currentFunction.name}"`,
        `[soroban-vm] [info] Invoking "${currentFunction.name}" with arguments: ${JSON.stringify(currentArgs)}`,
      ];

      let success = true;
      let errorCode = '';
      let errorMessage = '';
      let cpuConsumed = Math.floor(1000000 + Math.random() * 2000000); // base CPU
      let ramConsumed = Math.floor(1024 + Math.random() * 2048); // base memory
      let readKeys: string[] = [`Instance/${currentContract.contractId.slice(0, 12)}`];
      let writeKeys: string[] = [];
      let emittedEvents: SimEvent[] = [];
      let returnValue = 'Result: void';

      // Mock specific logic scenarios to give real dry-run feedback
      if (missingSignatures.length > 0) {
        success = false;
        errorCode = 'ContractError::Unauthorized';
        errorMessage = `Invocation lacks required signature auth for: [${missingSignatures.join(', ')}]`;
        trace.push(`[soroban-vm] [error] Auth verification failed! Missing valid signature for account: [${missingSignatures.join(', ')}]`);
        trace.push(`[soroban-vm] [info] Rollback state changes — execution aborted.`);
      } else {
        trace.push(`[soroban-vm] [info] Validating authorization credentials... Success!`);

        // Check for specific argument errors
        if (currentContract.id === 'token') {
          if (currentFunction.name === 'transfer') {
            const amount = parseFloat(currentArgs.amount || '0');
            const fromAcc = currentArgs.from;
            const toAcc = currentArgs.to;

            if (isNaN(amount) || amount <= 0) {
              success = false;
              errorCode = 'ContractError::InvalidAmount';
              errorMessage = 'Amount to transfer must be greater than zero';
              trace.push(`[soroban-vm] [error] Runtime Reverted: amount must be > 0. Received: ${amount}`);
            } else if (fromAcc === toAcc) {
              success = false;
              errorCode = 'ContractError::SelfTransfer';
              errorMessage = 'Cannot transfer tokens to yourself';
              trace.push(`[soroban-vm] [error] Runtime Reverted: self-transfer is forbidden`);
            } else if (amount > 100000) {
              success = false;
              errorCode = 'ContractError::InsufficientBalance';
              errorMessage = `Account "${fromAcc}" has insufficient balance. Required: ${amount}`;
              trace.push(`[soroban-vm] [error] Runtime Reverted: Insufficient balance. Balance check failed.`);
            } else {
              // Success Case
              cpuConsumed += Math.floor(amount * 10);
              ramConsumed += 50;
              readKeys.push(`Balance/${fromAcc}`, `Balance/${toAcc}`);
              writeKeys.push(`Balance/${fromAcc}`, `Balance/${toAcc}`);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['transfer', fromAcc, toAcc],
                topicLabel: 'token.transfer',
                valuePreview: `${amount.toLocaleString()} tokens moved from ${fromAcc} to ${toAcc}`,
              });
              returnValue = 'Result: true';
              trace.push(`[soroban-vm] [info] Invoking internal transfer ledger update`);
              trace.push(`[soroban-vm] [info] Deducting balance from ${fromAcc} (-${amount})`);
              trace.push(`[soroban-vm] [info] Adding balance to ${toAcc} (+${amount})`);
              trace.push(`[soroban-vm] [info] Event emitted: transfer [${fromAcc}, ${toAcc}]`);
            }
          } else if (currentFunction.name === 'mint') {
            const amount = parseFloat(currentArgs.amount || '0');
            const toAcc = currentArgs.to;
            if (isNaN(amount) || amount <= 0) {
              success = false;
              errorCode = 'ContractError::InvalidAmount';
              errorMessage = 'Amount to mint must be greater than zero';
              trace.push(`[soroban-vm] [error] Runtime Reverted: invalid mint amount`);
            } else {
              cpuConsumed += 80000;
              readKeys.push(`Balance/${toAcc}`);
              writeKeys.push(`Balance/${toAcc}`);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['mint', toAcc],
                topicLabel: 'token.mint',
                valuePreview: `Minted ${amount.toLocaleString()} tokens into ${toAcc}`,
              });
              returnValue = `Result: ${amount}`;
              trace.push(`[soroban-vm] [info] Minting ${amount} units to recipient address`);
            }
          } else if (currentFunction.name === 'burn') {
            const amount = parseFloat(currentArgs.amount || '0');
            const fromAcc = currentArgs.from;
            if (isNaN(amount) || amount <= 0) {
              success = false;
              errorCode = 'ContractError::InvalidAmount';
              errorMessage = 'Amount to burn must be greater than zero';
              trace.push(`[soroban-vm] [error] Runtime Reverted: invalid burn amount`);
            } else {
              cpuConsumed += 50000;
              readKeys.push(`Balance/${fromAcc}`);
              writeKeys.push(`Balance/${fromAcc}`);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['burn', fromAcc],
                topicLabel: 'token.burn',
                valuePreview: `Burned ${amount.toLocaleString()} tokens from ${fromAcc}`,
              });
              returnValue = 'Result: void';
              trace.push(`[soroban-vm] [info] Burning ${amount} units from source balance`);
            }
          } else if (currentFunction.name === 'approve') {
            const amount = parseFloat(currentArgs.amount || '0');
            const fromAcc = currentArgs.from;
            const spender = currentArgs.spender;
            cpuConsumed += 40000;
            readKeys.push(`Allowance/${fromAcc}/${spender}`);
            writeKeys.push(`Allowance/${fromAcc}/${spender}`);
            emittedEvents.push({
              contractId: currentContract.contractId,
              topics: ['approve', fromAcc, spender],
              topicLabel: 'allowance.approved',
              valuePreview: `Allowance of ${amount.toLocaleString()} approved for ${spender} by ${fromAcc}`,
            });
            returnValue = 'Result: true';
            trace.push(`[soroban-vm] [info] Storing approval key for spender ${spender}`);
          }
        } else if (currentContract.id === 'nft') {
          if (currentFunction.name === 'mint_nft') {
            const tokenId = parseInt(currentArgs.tokenId || '0');
            if (tokenId === 99) {
              success = false;
              errorCode = 'ContractError::TokenAlreadyExists';
              errorMessage = `NFT tokenId ${tokenId} has already been minted`;
              trace.push(`[soroban-vm] [error] Runtime Reverted: token identity collision for ID: ${tokenId}`);
            } else {
              cpuConsumed += 120000;
              readKeys.push(`NFT/${tokenId}`);
              writeKeys.push(`NFT/${tokenId}`);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['nft_mint', currentArgs.to, String(tokenId)],
                topicLabel: 'nft.minted',
                valuePreview: `NFT tokenId #${tokenId} minted to ${currentArgs.to}`,
              });
              returnValue = `Result: ${tokenId}`;
              trace.push(`[soroban-vm] [info] Successfully registered unique token #${tokenId}`);
            }
          } else if (currentFunction.name === 'transfer_nft') {
            const tokenId = parseInt(currentArgs.tokenId || '0');
            cpuConsumed += 95000;
            readKeys.push(`NFT/${tokenId}`);
            writeKeys.push(`NFT/${tokenId}`);
            emittedEvents.push({
              contractId: currentContract.contractId,
              topics: ['nft_transfer', currentArgs.from, currentArgs.to, String(tokenId)],
              topicLabel: 'nft.transferred',
              valuePreview: `NFT #${tokenId} transferred from ${currentArgs.from} to ${currentArgs.to}`,
            });
            returnValue = 'Result: true';
            trace.push(`[soroban-vm] [info] Ownership update on token #${tokenId}`);
          }
        } else if (currentContract.id === 'defi') {
          if (currentFunction.name === 'swap') {
            const amountIn = parseFloat(currentArgs.amountIn || '0');
            const minAmountOut = parseFloat(currentArgs.minAmountOut || '0');
            if (amountIn > 50000) {
              success = false;
              errorCode = 'ContractError::InsufficientLiquidity';
              errorMessage = 'Slippage limit reached: Pool reserves insufficient for swap size';
              trace.push(`[soroban-vm] [error] Runtime Reverted: swap pricing model returned slippage exceeding constraints`);
            } else {
              cpuConsumed += 350000;
              ramConsumed += 512;
              readKeys.push('AMM/PoolReserves', 'AMM/FeeCalculator');
              writeKeys.push('AMM/PoolReserves');
              const outAmount = Math.floor(amountIn * 0.98);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['swap', currentArgs.from],
                topicLabel: 'swap.executed',
                valuePreview: `Exchanged ${amountIn.toLocaleString()} Token A for ${outAmount.toLocaleString()} Token B (fees: 15 Token A)`,
              });
              returnValue = `Result: ${outAmount}`;
              trace.push(`[soroban-vm] [info] Evaluating constant-product formula: x * y = k`);
              trace.push(`[soroban-vm] [info] Estimated swap output: ${outAmount} Token B (min expected: ${minAmountOut})`);
            }
          } else if (currentFunction.name === 'add_liquidity') {
            const amountA = parseFloat(currentArgs.amountA || '0');
            const amountB = parseFloat(currentArgs.amountB || '0');
            cpuConsumed += 410000;
            readKeys.push('AMM/PoolReserves');
            writeKeys.push('AMM/PoolReserves', `Balance/${currentArgs.from}`);
            emittedEvents.push({
              contractId: currentContract.contractId,
              topics: ['add_liquidity', currentArgs.from],
              topicLabel: 'liquidity.added',
              valuePreview: `${currentArgs.from} deposited ${amountA.toLocaleString()} A and ${amountB.toLocaleString()} B. Minted liquidity shares.`,
            });
            returnValue = 'Result: true';
            trace.push(`[soroban-vm] [info] Calculating pool ratio coefficient...`);
          }
        } else if (currentContract.id === 'escrow') {
          if (currentFunction.name === 'create_escrow') {
            const amount = parseFloat(currentArgs.amount || '0');
            cpuConsumed += 150000;
            readKeys.push('Escrow/Counter');
            writeKeys.push('Escrow/Counter', 'Escrow/Vault/101');
            emittedEvents.push({
              contractId: currentContract.contractId,
              topics: ['escrow_created', currentArgs.buyer, currentArgs.seller],
              topicLabel: 'escrow.created',
              valuePreview: `Escrow #101 created by ${currentArgs.buyer} for ${currentArgs.seller} with ${amount.toLocaleString()} XLM`,
            });
            returnValue = 'Result: 101_u32';
            trace.push(`[soroban-vm] [info] Registering vault #101 with arbiter referee ${currentArgs.arbiter}`);
          } else if (currentFunction.name === 'claim_escrow') {
            const escrowId = parseInt(currentArgs.escrowId || '0');
            cpuConsumed += 90000;
            readKeys.push(`Escrow/Vault/${escrowId}`);
            writeKeys.push(`Escrow/Vault/${escrowId}`);
            emittedEvents.push({
              contractId: currentContract.contractId,
              topics: ['escrow_release', String(escrowId)],
              topicLabel: 'escrow.released',
              valuePreview: `Escrow #${escrowId} released to recipient ${currentArgs.recipient}`,
            });
            returnValue = 'Result: true';
            trace.push(`[soroban-vm] [info] Arbiter approved claim request. Resolving vault assets to recipient.`);
          } else if (currentFunction.name === 'refund_escrow') {
            const escrowId = parseInt(currentArgs.escrowId || '0');
            
            // Environment interaction check: If sequence is low or time is low, simulate "Lock period not expired"
            if (envConfig.ledgerTimestamp < Math.floor(Date.now() / 1000) + 100) {
              success = false;
              errorCode = 'ContractError::LockPeriodNotExpired';
              errorMessage = 'Escrow refund rejected: Timelock is still active';
              trace.push(`[soroban-vm] [error] Runtime Reverted: refund failed. Lock period expiration not reached. Current: ${envConfig.ledgerTimestamp}`);
            } else {
              cpuConsumed += 80000;
              readKeys.push(`Escrow/Vault/${escrowId}`);
              writeKeys.push(`Escrow/Vault/${escrowId}`);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['escrow_refund', String(escrowId)],
                topicLabel: 'escrow.refunded',
                valuePreview: `Escrow #${escrowId} refunded to buyer`,
              });
              returnValue = 'Result: true';
              trace.push(`[soroban-vm] [info] Timelock verification passed. Refunding locked assets back to buyer.`);
            }
          }
        } else if (currentContract.id === 'vesting') {
          if (currentFunction.name === 'create_vesting') {
            const amount = parseFloat(currentArgs.amount || '0');
            cpuConsumed += 180000;
            readKeys.push('Vesting/Counter');
            writeKeys.push('Vesting/Counter', 'Vesting/Schedule/12');
            emittedEvents.push({
              contractId: currentContract.contractId,
              topics: ['vesting_created', currentArgs.beneficiary],
              topicLabel: 'vesting.created',
              valuePreview: `Vesting schedule #12 created for ${currentArgs.beneficiary} with ${amount.toLocaleString()} tokens`,
            });
            returnValue = 'Result: 12_u32';
            trace.push(`[soroban-vm] [info] Storing vesting parameters: cliff ${currentArgs.cliffDays} days, duration ${currentArgs.durationDays} days`);
          } else if (currentFunction.name === 'claim_vested') {
            const vestingId = parseInt(currentArgs.vestingId || '0');
            
            // Check for cliff time constraint
            if (envConfig.ledgerSeq < 12940300) { // Simulate seq-based cliff
              success = false;
              errorCode = 'ContractError::CliffNotReached';
              errorMessage = 'Beneficiary cannot claim vesting shares prior to cliff sequence expiration';
              trace.push(`[soroban-vm] [error] Runtime Reverted: cliff check failed. Required ledger sequence: 12940300. Current: ${envConfig.ledgerSeq}`);
            } else {
              cpuConsumed += 110000;
              readKeys.push(`Vesting/Schedule/${vestingId}`);
              writeKeys.push(`Vesting/Schedule/${vestingId}`);
              emittedEvents.push({
                contractId: currentContract.contractId,
                topics: ['vesting_claim', currentArgs.beneficiary, String(vestingId)],
                topicLabel: 'vesting.claimed',
                valuePreview: `Vesting schedule #${vestingId} release triggered. Transferred 25,000 shares to ${currentArgs.beneficiary}`,
              });
              returnValue = 'Result: 25000_i128';
              trace.push(`[soroban-vm] [info] Cliff check passed. Calculating vesting payout schedule index...`);
            }
          }
        }
      }

      // Add final trace logs and calculate fees
      if (success) {
        trace.push(`[soroban-vm] [info] Execution completed successfully.`);
        trace.push(`[soroban-vm] [info] CPU Instructions Consumed: ${cpuConsumed.toLocaleString()}`);
        trace.push(`[soroban-vm] [info] Memory (RAM) Utilized: ${ramConsumed} KB`);
        trace.push(`[soroban-vm] [info] Final contract output: "${returnValue}"`);
        trace.push(`[soroban-vm] [info] Commit transaction proposal... Done.`);
      }

      // Estimate total fees in Stroops
      const baseFee = envConfig.feeRate;
      const footprintCost = (readKeys.length * 150) + (writeKeys.length * 400);
      const instructionCost = Math.floor(cpuConsumed / 10000);
      const estimatedFee = baseFee + footprintCost + instructionCost;

      setSimulationResult({
        success,
        errorCode,
        errorMessage,
        cpuConsumed,
        ramConsumed,
        estimatedFee,
        readKeys,
        writeKeys,
        emittedEvents,
        returnValue,
        traceLogs: trace,
      });

      setIsSimulating(false);
    }, 900); // 900ms simulated execution time for visual delight
  };

  return {
    selectedContractId,
    selectedContract,
    selectedFunctionName,
    selectedFunction,
    args,
    authSignatures,
    envConfig,
    isSimulating,
    simulationResult,
    handleSelectContract,
    handleSelectFunction,
    handleArgChange,
    toggleAuthSignature,
    handleEnvChange,
    resetConfig,
    runSimulation,
  };
};
