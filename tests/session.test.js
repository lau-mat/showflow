import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

import { FakeWebSocket, installFakeBrowser, sessionState, show } from "./helpers/fake_browser.js";

const { storage, fakeElement } = installFakeBrowser();

const { SessionSocket } = await import("../frontend/shared/js/session_socket.ts");
const client = await import("../frontend/session/js/client.ts");
const [clientSocket] = FakeWebSocket.instances;
await import("../frontend/session/js/session.ts");
const controllerSocket = FakeWebSocket.instances[1];

test("session socket connects to the page host, requests data on open and tags emitted commands", () => {
    const socket = new SessionSocket();
    const raw = FakeWebSocket.instances.at(-1);
    assert.equal(raw.url, "ws://192.168.1.20:23123");
    raw.onopen();
    assert.deepEqual(raw.sent, [{ command: "get_show_data" }, { command: "get_session_data" }]);
    socket.emit("jump_to_cue", { cue_number: 3 });
    assert.deepEqual(raw.sent.at(-1), { cue_number: 3, command: "jump_to_cue" });
});

test("session socket routes messages by command and rejects unknown ones", () => {
    const socket = new SessionSocket();
    const raw = FakeWebSocket.instances.at(-1);
    const received = [];
    socket.on("cue_change", message => received.push(message));
    raw.receive({ command: "cue_change", current_cue: 2, next_cue: null, show_started_at: 1, cue_started_at: 2 });
    assert.equal(received.length, 1);
    assert.equal(received[0].current_cue, 2);
    assert.throws(() => raw.receive({ command: "nope" }), /unknown command: nope/);
});

test("controller page loads outside Tauri and only needs the window API when toggling fullscreen", () => {
    controllerSocket.receive({ command: "get_show_data", data: show });
    assert.equal(fakeElement("ShowName").innerText, "Gala");
    assert.equal(fakeElement("itemCounter").innerText, "2 Items");

    fakeElement("btnNextCue").listeners.click();
    fakeElement("previousCue").listeners.click();
    assert.deepEqual(controllerSocket.sent.slice(-2), [{ command: "next_cue" }, { command: "previous_cue" }]);
    assert.throws(() => controllerSocket.receive({ command: "error", message: "boom" }), /Backend error: boom/);
});

test("client restores a previously joined user, including their client id", async () => {
    storage.set("client_id", "abc");
    clientSocket.receive({
        command: "get_session_data",
        session: sessionState([{ client_id: "abc", client_name: "Sam", client_role: 7 }])
    });
    clientSocket.receive({ command: "get_show_data", data: show });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(client.session.userName, "Sam");
    assert.equal(client.session.roleId, 7);
    assert.equal(client.session.clientId, "abc");
});
