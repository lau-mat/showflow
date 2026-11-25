import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";
import { FakeWebSocket, installFakeBrowser, show } from "./helpers/fake_browser.js";

const { storage, navigations } = installFakeBrowser();
const gates = [];
globalThis.__generateDynamic = definitions => { gates.push(definitions); return {}; };
const prompts = [];
globalThis.__dynamicPrompt = async options => { prompts.push(options); return { accessName: "Eve", accessCode: "JoinMe!23" }; };
globalThis.document.body = {};

await import("../frontend/session/js/client.ts");
const [socket] = FakeWebSocket.instances;
const settle = () => new Promise(resolve => setImmediate(resolve));

const none = { open: false, code: false, request: false, account: false };
const welcome = { command: "welcome", protocol: 2, session_id: "s2", show_id: "show-1", nonce: "n", policy: none, view_policy: { ...none, code: true } };

test("a protected session shows only the way in, and asks for the code at once", async () => {
    socket.receive(welcome);
    await settle();
    assert.ok(gates[0].some(item => item.id === "viewGate"));
    assert.ok(!JSON.stringify(gates[0]).includes(show.show.name), "nothing about the show");
    assert.equal(prompts[0].title, "Join the session");
    assert.deepEqual(socket.sent, [{ command: "request_view", claim: { method: "code", name: "Eve", code: "JoinMe!23" } }]);
});

test("being let in keeps the token, and a reload comes back with it without asking", async () => {
    socket.receive({ command: "view_status", state: "granted", reason: null, resume_token: "view-1" });
    assert.equal(storage.get("showflow.view.s2"), "view-1");
    socket.sent.length = 0;
    prompts.length = 0;
    socket.receive(welcome);
    await settle();
    assert.deepEqual(socket.sent, [{ command: "request_view", claim: { method: "resume", token: "view-1" } }]);
    assert.equal(prompts.length, 0);
});

test("a token that no longer works is dropped and the page stays at the gate", () => {
    navigations.length = 0;
    socket.receive({ command: "view_status", state: "denied", reason: "Your access has ended", resume_token: null });
    assert.equal(storage.get("showflow.view.s2"), undefined);
    assert.deepEqual(navigations, [], "nothing was shown, so nothing to clear");
});

test("being put out after seeing the show starts over on a clean page", () => {
    navigations.length = 0;
    storage.set("showflow.view.s2", "view-2");
    socket.receive({ command: "get_show_data", data: show });
    socket.receive({ command: "view_status", state: "revoked", reason: "The host removed you from the session", resume_token: null });
    assert.equal(storage.get("showflow.view.s2"), undefined);
    assert.deepEqual(navigations, [["reload"]]);
});

test("an open session has no gate", async () => {
    gates.length = 0;
    prompts.length = 0;
    socket.sent.length = 0;
    socket.receive({ ...welcome, view_policy: { ...none, open: true } });
    await settle();
    assert.deepEqual([gates.length, prompts.length, socket.sent.length], [0, 0, 0]);
});
