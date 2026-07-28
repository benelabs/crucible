/**
 * Offline storage and optimistic UI helper using IndexedDB
 */

const DB_NAME = 'CrucibleOfflineDB';
const DB_VERSION = 1;
const STORE_DASHBOARD = 'dashboard_state';
const STORE_PENDING_MUTATIONS = 'pending_mutations';

function openDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);

    request.onupgradeneeded = (event) => {
      const db = (event.target as IDBOpenDBRequest).result;
      if (!db.objectStoreNames.contains(STORE_DASHBOARD)) {
        db.createObjectStore(STORE_DASHBOARD, { keyPath: 'id' });
      }
      if (!db.objectStoreNames.contains(STORE_PENDING_MUTATIONS)) {
        db.createObjectStore(STORE_PENDING_MUTATIONS, { keyPath: 'id', autoIncrement: true });
      }
    };

    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export async function saveDashboardCache(id: string, data: unknown): Promise<void> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_DASHBOARD, 'readwrite');
    const store = tx.objectStore(STORE_DASHBOARD);
    store.put({ id, data, timestamp: Date.now() });
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

export async function getDashboardCache<T = unknown>(id: string): Promise<T | null> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_DASHBOARD, 'readonly');
    const store = tx.objectStore(STORE_DASHBOARD);
    const request = store.get(id);
    request.onsuccess = () => {
      resolve(request.result ? (request.result.data as T) : null);
    };
    request.onerror = () => reject(request.error);
  });
}

export async function addPendingMutation(action: string, payload: unknown): Promise<number> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_PENDING_MUTATIONS, 'readwrite');
    const store = tx.objectStore(STORE_PENDING_MUTATIONS);
    const request = store.add({ action, payload, timestamp: Date.now() });
    request.onsuccess = () => resolve(request.result as number);
    request.onerror = () => reject(request.error);
  });
}

export async function getPendingMutations(): Promise<Array<{ id: number; action: string; payload: unknown }>> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_PENDING_MUTATIONS, 'readonly');
    const store = tx.objectStore(STORE_PENDING_MUTATIONS);
    const request = store.getAll();
    request.onsuccess = () => resolve(request.result || []);
    request.onerror = () => reject(request.error);
  });
}

export async function clearPendingMutation(id: number): Promise<void> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_PENDING_MUTATIONS, 'readwrite');
    const store = tx.objectStore(STORE_PENDING_MUTATIONS);
    store.delete(id);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}
