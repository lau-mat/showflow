// Minimal browser globals for importing the session pages under node:test.
export class FakeWebSocket {
    static instances = [];
    sent = [];
    constructor(url) { this.url = url; FakeWebSocket.instances.push(this); }
    send(message) { this.sent.push(JSON.parse(message)); }
    receive(message) { this.onmessage({ data: JSON.stringify(message) }); }
}

export function installFakeBrowser() {
    const storage = new Map();
    const elements = new Map();
    const ids = ["ShowName", "itemCounter", "btnNextCue", "previousCue", "releaseCaller", "cueChangedBy", "callerList"];
    const navigations = [];
    const fakeElement = id => elements.get(id) ?? elements.set(id, {
        id, innerText: "", textContent: "", listeners: {},
        addEventListener(type, listener) { this.listeners[type] = listener; },
        replaceChildren() {}
    }).get(id);

    globalThis.WebSocket = FakeWebSocket;
    globalThis.window = {
        location: {
            hostname: "192.168.1.20",
            // The socket port is given explicitly, so no /session-info request is needed.
            search: "?ws=47123",
            assign: url => navigations.push(["assign", url]),
            replace: url => navigations.push(["replace", url]),
            reload: () => navigations.push(["reload"])
        },
        localStorage: {
            getItem: key => storage.get(key) ?? null,
            setItem: (key, value) => storage.set(key, value),
            removeItem: key => storage.delete(key)
        }
    };
    globalThis.document = {
        readyState: "complete",
        addEventListener() {},
        getElementById: id => (ids.includes(id) ? fakeElement(id) : null),
        querySelectorAll: () => [],
        querySelector: () => null
    };
    globalThis.setInterval = () => 0;
    globalThis.clearInterval = () => {};
    globalThis.__currentWindow = undefined;
    globalThis.__generateDynamic = () => ({});
    globalThis.__dynamicGenerator = () => ({ replaceChildren() {}, remove() {} });
    return { storage, fakeElement, navigations };
}

export const show = {
    show: { id: "show-1", name: "Gala", time: 1_800_000_000, role: "owner" },
    roles: [{ id: "7", show_id: "show-1", name: "Sound" }],
    lines: [
        { id: "line-1", position: "a", name: "Opening", note: "", duration: 60, absolute_time: null },
        { id: "line-2", position: "b", name: "Speech", note: "", duration: 120, absolute_time: null }
    ],
    comments: [],
    rev: 0
};

export const sessionState = (joined_users = []) => ({
    session_id: "s", show_id: "show-1", is_active: true, current_cue: null,
    show_started_at: null, cue_started_at: null, joined_users
});
