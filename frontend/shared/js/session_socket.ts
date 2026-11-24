import type { ClientMessages, ServerMessages } from "./session_types";

type Handler<K extends keyof ServerMessages> = (message: ServerMessages[K] & { command: K }) => void;

/** Typed websocket wrapper shared by the session controller and the crew client. */
export class SessionSocket {
    readonly socket: WebSocket;
    private readonly routes: { [K in keyof ServerMessages]?: Handler<K> } = {};

    constructor(port = 23123) {
        // Auto-detect host IP address
        this.socket = new WebSocket(`ws://${window.location.hostname}:${port}`);

        this.socket.onopen = () => {
            this.emit("get_show_data");
            this.emit("get_session_data");
        };
        this.socket.onmessage = event => {
            const data = JSON.parse(event.data) as { command: keyof ServerMessages };
            const handler = this.routes[data.command] as ((message: unknown) => void) | undefined;
            if (handler === undefined) throw new Error(`Received unknown command: ${data.command}`);
            handler(data);
        };
        this.socket.onclose = () => console.log("websocket closed");
    }

    on<K extends keyof ServerMessages>(command: K, handler: Handler<K>): void {
        (this.routes as Record<string, unknown>)[command] = handler;
    }

    emit<K extends keyof ClientMessages>(command: K, data?: ClientMessages[K]): void {
        this.socket.send(JSON.stringify({ ...data, command }));
    }
}
