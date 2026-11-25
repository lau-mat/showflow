// Reading what someone pasted or typed to join a session on another computer.

export interface JoinTarget {
    host: string;
    port: number;
}

/**
 * Accepts the join address from a session panel or QR code ("http://192.168.1.5:47123"), the same
 * without "http://", or a page link on it. The port is required: every session has its own.
 */
export function parseJoinLink(input: string): JoinTarget | null {
    const text = input.trim();
    if (!text || /\s/.test(text)) return null;
    const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `http://${text}`;
    let url: URL;
    try {
        url = new URL(withScheme);
    } catch {
        return null;
    }
    // Sessions are plain HTTP on the venue network. (A missing port would silently mean 80.)
    if (url.protocol !== "http:" || url.username || url.password) return null;
    const port = Number(url.port);
    if (!url.hostname || !Number.isInteger(port) || port <= 0) return null;
    return { host: url.hostname, port };
}

/** Where the ShowFlow app opens its own copy of the crew page for a session elsewhere. */
export function joinPagePath(target: JoinTarget): string {
    return `session/client.html?${new URLSearchParams({ host: target.host, port: String(target.port) })}`;
}

export const formatTarget = (target: JoinTarget): string => `${target.host}:${target.port}`;
