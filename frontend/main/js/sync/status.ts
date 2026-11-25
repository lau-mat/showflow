/** What the sync badge shows. */
export type SyncPhase = "idle" | "syncing" | "offline" | "error" | "paused";

export interface SyncStatus {
    phase: SyncPhase;
    /** Edits made here that the cloud has not confirmed yet. */
    pending: number;
    lastSyncedAt: number | null;
    /** Something the user should know (an edit the server refused, a failure). */
    message: string | null;
}

export class SyncStatusStore {
    private current: SyncStatus = { phase: "paused", pending: 0, lastSyncedAt: null, message: null };
    private readonly listeners = new Set<(status: SyncStatus) => void>();

    get status(): SyncStatus {
        return this.current;
    }

    update(patch: Partial<SyncStatus>): void {
        this.current = { ...this.current, ...patch };
        for (const listener of [...this.listeners]) listener(this.current);
    }

    subscribe(listener: (status: SyncStatus) => void): () => void {
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
    }
}
