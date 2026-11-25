import { PROTOCOL_VERSION, type ClientMessages, type ServerMessages } from "./session_types";

type Handler<K extends keyof ServerMessages> = (message: ServerMessages[K] & { command: K }) => void;

/** Every command a server may send; a page ignores the ones it has no use for. */
const SERVER_COMMANDS = new Set<string>([
    "welcome", "policy", "view_status", "caller_status", "callers", "join_session", "get_show_data", "get_session_data", "cue_change", "error"
]);

export interface SessionAddress {
    host: string;
    wsPort: number;
}

/** Set by the app in the host computer's own caller window. */
export interface HostContext {
    sessionId: string;
    host: string;
    ws: number;
    isHost: boolean;
}

declare global {
    interface Window {
        __SHOWFLOW_SESSION__?: HostContext;
    }
}

/** The host runs a different protocol version: one side needs updating. */
export class ProtocolMismatch extends Error {
    constructor() {
        super("This ShowFlow session uses a different version. Update ShowFlow on this device and on the host.");
        this.name = "ProtocolMismatch";
    }
}

/**
 * Where the session's websocket is. In order: what the host app injected; an explicit `?ws=`
 * (used for `npm run dev:session`); otherwise the session's web server says so
 * (`/session-info`, on the page's own server, or on `?host=&port=` when the ShowFlow app opens
 * its bundled pages for a session running somewhere else).
 */
export async function resolveSessionAddress(
    location: Pick<Location, "hostname" | "search"> = window.location,
    injected: HostContext | undefined = window.__SHOWFLOW_SESSION__,
    request: typeof fetch = (input, init) => fetch(input, init)
): Promise<SessionAddress> {
    if (injected) return { host: injected.host, wsPort: injected.ws };
    const params = new URLSearchParams(location.search);
    const remoteHost = params.get("host");
    const explicit = Number(params.get("ws"));
    if (explicit > 0) return { host: remoteHost ?? location.hostname, wsPort: explicit };
    const base = remoteHost ? `http://${remoteHost}:${Number(params.get("port"))}` : "";
    const response = await request(`${base}/session-info`, { cache: "no-store" });
    if (!response.ok) throw new Error("This address has no running ShowFlow session.");
    const info = await response.json() as { protocol: number; ws_port: number };
    if (info.protocol !== PROTOCOL_VERSION) throw new ProtocolMismatch();
    return { host: remoteHost ?? location.hostname, wsPort: info.ws_port };
}

/** Typed websocket wrapper shared by the caller view and the crew client. */
export class SessionSocket {
    readonly socket: WebSocket;
    private readonly routes: { [K in keyof ServerMessages]?: Handler<K> } = {};
    private closeListener: (() => void) | null = null;

    constructor(address: SessionAddress) {
        this.socket = new WebSocket(`ws://${address.host}:${address.wsPort}`);

        this.socket.onmessage = event => {
            const data = JSON.parse(event.data) as { command: keyof ServerMessages };
            const handler = this.routes[data.command] as ((message: unknown) => void) | undefined;
            if (handler) handler(data);
            else if (!SERVER_COMMANDS.has(data.command)) throw new Error(`Received unknown command: ${data.command}`);
        };
        this.socket.onclose = () => this.closeListener?.();
    }

    static async open(): Promise<SessionSocket> {
        return new SessionSocket(await resolveSessionAddress());
    }

    on<K extends keyof ServerMessages>(command: K, handler: Handler<K>): void {
        (this.routes as Record<string, unknown>)[command] = handler;
    }

    onClose(listener: () => void): void {
        this.closeListener = listener;
    }

    emit<K extends keyof ClientMessages>(command: K, data?: ClientMessages[K]): void {
        this.socket.send(JSON.stringify({ ...data, command }));
    }
}
