import { ApiError, NetworkError } from "../api/types";

/** What the server announces on the live stream (backend/src/sync_stream.rs). */
export type LiveEvent =
    | { type: "ready" }
    | { type: "show"; showId: string; rev: number }
    | { type: "access" }
    | { type: "resync" };

/**
 * Splits a server-sent-events byte stream into `{event, data}` messages. Not `EventSource`:
 * that cannot send an `Authorization` header, and apps are not same-origin with the API.
 */
export async function* parseSse(body: ReadableStream<Uint8Array>): AsyncGenerator<{ event: string; data: string }> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    try {
        for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
            let end: number;
            while ((end = buffer.indexOf("\n\n")) !== -1) {
                const block = buffer.slice(0, end);
                buffer = buffer.slice(end + 2);
                let event = "message";
                const data: string[] = [];
                for (const line of block.split("\n")) {
                    if (line.startsWith(":")) continue; // keep-alive comment
                    if (line.startsWith("event:")) event = line.slice(6).trim();
                    else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
                }
                if (data.length > 0 || event !== "message") yield { event, data: data.join("\n") };
            }
        }
    } finally {
        reader.releaseLock();
    }
}

export function toLiveEvent(message: { event: string; data: string }): LiveEvent | null {
    switch (message.event) {
        case "ready": return { type: "ready" };
        case "access": return { type: "access" };
        case "resync": return { type: "resync" };
        case "show": {
            try {
                const { show_id, rev } = JSON.parse(message.data) as { show_id: string; rev: number };
                return { type: "show", showId: show_id, rev };
            } catch {
                return null;
            }
        }
        default: return null;
    }
}

export interface LiveUpdatesDeps {
    /** Opens the stream with the current access token (refreshing it when needed). */
    open: (signal: AbortSignal) => Promise<Response>;
    session: { state: string; subscribe: (listener: (state: string) => void) => () => void };
    onEvent: (event: LiveEvent) => void;
    /** Injectable for tests. */
    sleep?: (ms: number) => Promise<void>;
}

const BACKOFF_START_MS = 1_000;
const BACKOFF_MAX_MS = 30_000;

/**
 * Keeps one live stream open while signed in, reconnecting with backoff. The server ends each
 * stream when its access token expires, which just means "reconnect with a fresh one".
 */
export class LiveUpdates {
    private readonly deps: LiveUpdatesDeps;
    private abort: AbortController | null = null;
    private running = false;
    private wake: (() => void) | null = null;

    constructor(deps: LiveUpdatesDeps) {
        this.deps = deps;
    }

    start(): void {
        if (this.running) return;
        this.running = true;
        // Signing in starts the stream (and out stops it) without waiting for a retry timer.
        this.deps.session.subscribe(() => {
            this.wake?.();
            if (this.deps.session.state !== "authenticated") this.abort?.abort();
        });
        void this.loop();
    }

    stop(): void {
        this.running = false;
        this.abort?.abort();
        this.wake?.();
    }

    private sleep(ms: number): Promise<void> {
        if (this.deps.sleep) return this.deps.sleep(ms);
        return new Promise<void>(resolve => {
            const timer = setTimeout(resolve, ms);
            this.wake = () => { clearTimeout(timer); resolve(); };
        }).then(() => { this.wake = null; });
    }

    private async loop(): Promise<void> {
        let backoff = BACKOFF_START_MS;
        while (this.running) {
            if (this.deps.session.state !== "authenticated") {
                await this.sleep(BACKOFF_MAX_MS);
                continue;
            }
            this.abort = new AbortController();
            try {
                const response = await this.deps.open(this.abort.signal);
                if (!response.body) throw new NetworkError();
                backoff = BACKOFF_START_MS;
                for await (const message of parseSse(response.body)) {
                    const event = toLiveEvent(message);
                    if (event) this.deps.onEvent(event);
                }
                // The server closed the stream (token expiry): reconnect right away.
                await this.sleep(250);
            } catch (error) {
                if (!this.running) return;
                const rejected = error instanceof ApiError && error.isRejection;
                // Rejected (e.g. an expired login): the session will say so; do not hammer the server.
                await this.sleep(rejected ? BACKOFF_MAX_MS : backoff);
                backoff = Math.min(backoff * 2, BACKOFF_MAX_MS);
            }
        }
    }
}
