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
    const ids = ["ShowName", "itemCounter", "btnNextCue", "previousCue"];
    const fakeElement = id => elements.get(id) ?? elements.set(id, {
        id, innerText: "", textContent: "", listeners: {},
        addEventListener(type, listener) { this.listeners[type] = listener; },
        replaceChildren() {}
    }).get(id);

    globalThis.WebSocket = FakeWebSocket;
    globalThis.window = {
        location: { hostname: "192.168.1.20" },
        localStorage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) }
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
    globalThis.__dynamicGenerator = () => ({ replaceChildren() {} });
    return { storage, fakeElement };
}

export const show = {
    show: { id: 1, name: "Gala", time: 1_800_000_000 },
    roles: [{ id: 7, show_id: 1, name: "Sound" }],
    lines: [
        { id: 1, order: 1, name: "Opening", comment: "", duration: 60, absolute_time: null },
        { id: 2, order: 2, name: "Speech", comment: "", duration: 120, absolute_time: null }
    ],
    comments: []
};

export const sessionState = (joined_users = []) => ({
    session_id: "s", show_id: 1, is_active: true, current_cue: null,
    show_started_at: null, cue_started_at: null, joined_users
});
