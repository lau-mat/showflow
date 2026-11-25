import type { Change, Op, PullResponse, PushResponse } from "/shared/js/sync_types";
import type { Show } from "/shared/js/show_types";
import { ApiError, NetworkError, type SyncApi } from "../api/types";
import type { AuthSession } from "../auth/session";
import type { ChangeBus } from "./bus";
import type { SyncStatusStore } from "./status";

/** The slice of `RestClient` the engine needs. */
export interface SyncTransport {
    request<T>(path: string, options?: { body?: unknown; authorized?: boolean }): Promise<T>;
}

export interface SyncEngineDeps {
    session: Pick<AuthSession, "state" | "user" | "subscribe">;
    rest: SyncTransport;
    local: SyncApi;
    bus: ChangeBus;
    status: SyncStatusStore;
    /** Runs after a pull with the cloud's show list (apps: keeps caller access current). */
    afterPull?: (index: ServerShow[]) => Promise<void>;
    now?: () => number;
    /** Injectable for tests; defaults to the real timers. */
    setTimer?: (callback: () => void, ms: number) => unknown;
    clearTimer?: (handle: unknown) => void;
}

export interface ServerShow extends Show {
    rev: number;
    members?: string[];
    callers?: string[];
}

/** Ops per request: the server accepts up to 500, smaller batches keep retries cheap. */
const PUSH_BATCH = 100;
/** Edits are sent shortly after the user stops typing, not on every keystroke. */
const LOCAL_WRITE_DELAY_MS = 800;
/** Safety net for a missed live update; the live stream normally triggers pulls sooner. */
const POLL_MS = 30_000;

/**
 * Keeps the local database and the cloud converged: push what was edited here, pull what was
 * edited elsewhere. The app edits locally first (so it works offline); the cloud assigns the
 * order. Safe to trigger as often as you like: runs never overlap, and every step is
 * idempotent (ops carry ids, pulls resume from a per-show revision).
 */
export class SyncEngine {
    private readonly deps: SyncEngineDeps;
    private readonly now: () => number;
    private running: Promise<void> | null = null;
    private rerun = false;
    private timer: unknown = null;
    private poll: unknown = null;

    constructor(deps: SyncEngineDeps) {
        this.deps = deps;
        this.now = deps.now ?? Date.now;
    }

    /** Hooks the triggers: local edits, sign-in, and a slow poll. */
    start(): void {
        const { bus, session } = this.deps;
        bus.onLocalWrite(() => this.trigger(LOCAL_WRITE_DELAY_MS));
        session.subscribe(state => { if (state === "authenticated") this.trigger(0); });
        const setTimer = this.deps.setTimer ?? ((cb, ms) => setInterval(cb, ms));
        this.poll = setTimer(() => this.trigger(0), POLL_MS);
        this.trigger(0);
    }

    /** Runs a sync after `delayMs` (coalescing repeated triggers into one). */
    trigger(delayMs = 0): void {
        const setTimer = this.deps.setTimer ?? ((cb, ms) => setTimeout(cb, ms));
        const clearTimer = this.deps.clearTimer ?? (handle => clearTimeout(handle as number));
        if (this.timer !== null) clearTimer(this.timer);
        this.timer = setTimer(() => {
            this.timer = null;
            void this.syncNow();
        }, delayMs);
    }

    /** One full cycle. Calls while a cycle runs make it run once more when it ends. */
    syncNow(): Promise<void> {
        if (this.running) {
            this.rerun = true;
            return this.running;
        }
        this.running = (async () => {
            try {
                do {
                    this.rerun = false;
                    await this.cycle();
                } while (this.rerun);
            } finally {
                this.running = null;
            }
        })();
        return this.running;
    }

    private async cycle(): Promise<void> {
        const { session, local, status } = this.deps;
        const userId = session.user?.id;
        // Offline grace and signed-out states have no way to reach the cloud: wait.
        if (session.state !== "authenticated" || !userId) {
            status.update({ phase: "paused" });
            return;
        }
        status.update({ phase: "syncing" });
        try {
            await local.adoptAccount(userId);
            const notices: string[] = [];
            await this.push(notices);
            await this.pull();
            const { pending_total } = await local.state();
            status.update({
                phase: "idle",
                pending: pending_total,
                lastSyncedAt: this.now(),
                message: notices.length ? notices.join(" ") : null
            });
        } catch (error) {
            const pending = await local.state().then(state => state.pending_total, () => 0);
            if (error instanceof NetworkError) status.update({ phase: "offline", pending });
            else if (error instanceof ApiError && error.status === 401) status.update({ phase: "paused", pending });
            else status.update({ phase: "error", pending, message: error instanceof Error ? error.message : String(error) });
        }
    }

    /** Sends queued edits, oldest first, one show at a time. */
    private async push(notices: string[]): Promise<void> {
        const { local, rest, bus } = this.deps;
        for (;;) {
            const queued = await local.pending(PUSH_BATCH);
            if (queued.length === 0) return;
            const showId = queued[0].show_id;
            const ops: Op[] = [];
            for (const item of queued) {
                if (item.show_id !== showId) break;
                ops.push(item.op);
            }

            let response: PushResponse;
            try {
                response = await rest.request<PushResponse>(`shows/${showId}/ops`, { body: { ops }, authorized: true });
            } catch (error) {
                // The show is gone or we may no longer touch it: drop it rather than retry forever.
                if (error instanceof ApiError && [403, 404, 410].includes(error.status)) {
                    await local.forgetShow(showId);
                    notices.push("A show you were editing is no longer available to you.");
                    bus.emitExternal(null);
                    continue;
                }
                throw error;
            }

            // Applied, already known, and rejected are all final: none of them is sent again.
            await local.ack(showId, response.results.map(result => result.op_id));
            const rejected = response.results.flatMap(result => (result.status === "rejected" ? [result.error] : []));
            if (rejected.length > 0) {
                // The local copy ran ahead of what the cloud allows; re-download the show.
                await local.forgetShow(showId);
                notices.push(`Some changes were not accepted: ${[...new Set(rejected)].join("; ")}.`);
                bus.emitExternal(null);
            }
        }
    }

    /** Fetches every show that moved on in the cloud, and forgets shows that went away. */
    private async pull(): Promise<void> {
        const { local, rest, bus } = this.deps;
        const index = await rest.request<ServerShow[]>("shows", { authorized: true });
        const known = new Map((await local.state()).shows.map(show => [show.show_id, show]));
        const listChanged = { value: false };

        for (const show of index) {
            const have = known.get(show.id);
            if (have && show.rev <= have.server_rev && show.role === have.role) continue;
            let cursor = have?.server_rev ?? 0;
            let anything = !have || show.role !== have.role;
            for (;;) {
                const page = await rest.request<PullResponse>(`shows/${show.id}/changes?since=${cursor}`, { authorized: true });
                const last = page.changes.at(-1);
                cursor = page.more && last ? last.rev : page.rev;
                await local.applyChanges(show.id, page.changes as Change[], cursor, show.role);
                anything ||= page.changes.length > 0;
                if (!page.more || !last) break;
            }
            if (!have) listChanged.value = true;
            if (anything) bus.emitExternal(show.id);
        }

        // Synced here before, but no longer in the cloud's list: deleted, or access removed.
        const current = new Set(index.map(show => show.id));
        for (const [id, show] of known) {
            if (show.synced && !current.has(id)) {
                await local.forgetShow(id);
                listChanged.value = true;
                bus.emitExternal(id);
            }
        }
        if (listChanged.value) bus.emitExternal(null);
        await this.deps.afterPull?.(index);
    }

    stop(): void {
        const clearTimer = this.deps.clearTimer ?? (handle => clearTimeout(handle as number));
        if (this.timer !== null) clearTimer(this.timer);
        if (this.poll !== null) (this.deps.clearTimer ?? (handle => clearInterval(handle as number)))(this.poll);
        this.timer = this.poll = null;
    }
}
