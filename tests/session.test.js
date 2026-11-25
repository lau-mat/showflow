import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

import { FakeWebSocket, installFakeBrowser, sessionState, show } from "./helpers/fake_browser.js";

const { storage, fakeElement, navigations } = installFakeBrowser();

const { SessionSocket } = await import("../frontend/shared/js/session_socket.ts");
const client = await import("../frontend/session/js/client.ts");
const [clientSocket] = FakeWebSocket.instances;
await import("../frontend/session/js/session.ts");
const controllerSocket = FakeWebSocket.instances[1];

test("session socket connects to the page host, asks for nothing by itself and tags emitted commands", () => {
    const socket = new SessionSocket({ host: "192.168.1.20", wsPort: 47123 });
    const raw = FakeWebSocket.instances.at(-1);
    assert.equal(raw.url, "ws://192.168.1.20:47123");
    // The host sends the show on its own once this device is in.
    raw.onopen?.();
    assert.deepEqual(raw.sent, []);
    socket.emit("jump_to_cue", { cue_number: 3 });
    assert.deepEqual(raw.sent.at(-1), { cue_number: 3, command: "jump_to_cue" });
});

test("session socket routes messages by command and rejects unknown ones", () => {
    const socket = new SessionSocket({ host: "192.168.1.20", wsPort: 47123 });
    const raw = FakeWebSocket.instances.at(-1);
    const received = [];
    socket.on("cue_change", message => received.push(message));
    raw.receive({ command: "cue_change", current_cue: 2, next_cue: null, show_started_at: 1, cue_started_at: 2 });
    assert.equal(received.length, 1);
    assert.equal(received[0].current_cue, 2);
    assert.throws(() => raw.receive({ command: "nope" }), /unknown command: nope/);
    // A command this page has no handler for is simply not its business.
    assert.doesNotThrow(() => raw.receive({ command: "callers", list: [] }));
});

const welcome = {
    command: "welcome", protocol: 2, session_id: "s1", show_id: "show-1", nonce: "n",
    policy: { open: false, code: true, request: false, account: false },
    view_policy: { open: true, code: false, request: false, account: false }
};

test("the caller view sends a device that is not a caller back to the join screen", () => {
    navigations.length = 0;
    controllerSocket.receive(welcome);
    assert.deepEqual(navigations, [["replace", "client.html?ws=47123"]]);
    assert.equal(fakeElement("btnNextCue").disabled, true);
});

test("a stored token makes the caller view resume, and the controls work only once granted", () => {
    navigations.length = 0;
    controllerSocket.sent.length = 0;
    storage.set("showflow.caller.s1", "token-1");
    controllerSocket.receive(welcome);
    assert.deepEqual(controllerSocket.sent, [{ command: "claim_caller", claim: { method: "resume", token: "token-1" } }]);
    assert.deepEqual(navigations, [], "no bounce while the server checks the token");

    controllerSocket.receive({ command: "caller_status", state: "granted", reason: null, resume_token: "token-1" });
    assert.equal(fakeElement("btnNextCue").disabled, false);
    controllerSocket.receive({ command: "callers", list: [{ name: "Ann", method: "code" }, { name: "Host", method: "host" }] });
    assert.equal(fakeElement("callerList").textContent, "Ann, Host");
});

test("controller page loads outside Tauri and only needs the window API when toggling fullscreen", () => {
    controllerSocket.receive({ command: "get_show_data", data: show });
    assert.equal(fakeElement("ShowName").innerText, "Gala");
    assert.equal(fakeElement("itemCounter").innerText, "2 Items");
    assert.throws(() => controllerSocket.receive({ command: "error", message: "boom" }), /Backend error: boom/);
});

test("Next and Previous say which cue the caller saw, so a stale press can be ignored", () => {
    controllerSocket.sent.length = 0;
    controllerSocket.receive({ command: "get_session_data", session: sessionState() });
    fakeElement("btnNextCue").listeners.click();
    controllerSocket.receive({ command: "cue_change", current_cue: 2, next_cue: 3, show_started_at: 1, cue_started_at: 2, by: "Ben" });
    fakeElement("previousCue").listeners.click();
    fakeElement("releaseCaller").listeners.click();
    assert.deepEqual(controllerSocket.sent, [
        { command: "next_cue", from_cue: null },
        { command: "previous_cue", from_cue: 2 },
        { command: "release_caller" }
    ]);
    assert.equal(fakeElement("cueChangedBy").textContent, "by Ben");
});

test("losing the caller role disables the controls, forgets the token and returns to the join screen", () => {
    navigations.length = 0;
    controllerSocket.receive({ command: "caller_status", state: "revoked", reason: "The caller code was changed", resume_token: null });
    assert.equal(fakeElement("btnNextCue").disabled, true);
    assert.equal(storage.get("showflow.caller.s1"), undefined);
    assert.deepEqual(navigations, [["replace", "client.html?ws=47123"]]);
});

test("client restores a previously joined user, including their client id", async () => {
    storage.set("client_id", "abc");
    clientSocket.receive({
        command: "get_session_data",
        session: sessionState([{ client_id: "abc", client_name: "Sam", client_role: 7, is_caller: false }])
    });
    clientSocket.receive({ command: "get_show_data", data: show });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(client.session.userName, "Sam");
    assert.equal(client.session.roleId, 7);
    assert.equal(client.session.clientId, "abc");
});
