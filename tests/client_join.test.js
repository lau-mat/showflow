import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";
import { FakeWebSocket, installFakeBrowser, sessionState, show } from "./helpers/fake_browser.js";

const { storage, navigations } = installFakeBrowser();
globalThis.__dynamicPrompt = async () => ({ userName: "Alex", roleId: "7" });

const client = await import("../frontend/session/js/client.ts");
const [socket] = FakeWebSocket.instances;

test("a new client joins with the chosen role and stores the id the server confirms", async () => {
    socket.receive({ command: "get_session_data", session: sessionState() });
    socket.receive({ command: "get_show_data", data: show });
    await new Promise(resolve => setImmediate(resolve));

    assert.deepEqual(socket.sent.at(-1), { user_name: "Alex", role_id: "7", command: "join_session" });
    assert.equal(client.session.roleId, "7");

    // Someone else joining must not be adopted as our identity.
    socket.receive({ command: "join_session", client_id: "other", client_name: "Sam", client_role: "7" });
    assert.equal(storage.get("client_id"), undefined);

    socket.receive({ command: "join_session", client_id: "mine", client_name: "Alex", client_role: "7" });
    assert.equal(storage.get("client_id"), "mine");
    assert.equal(client.session.clientId, "mine");
    assert.equal(client.session.sessionData.joined_users.length, 2);
});

test("becoming caller from the crew page moves the device to the caller view", () => {
    socket.receive({
        command: "welcome", protocol: 2, session_id: "s9", show_id: "show-1", nonce: "n",
        policy: { open: true, code: false, request: false, account: false },
        view_policy: { open: true, code: false, request: false, account: false }
    });
    socket.receive({ command: "caller_status", state: "granted", reason: null, resume_token: "tok" });
    assert.equal(storage.get("showflow.caller.s9"), "tok");
    assert.deepEqual(navigations.at(-1), ["assign", "session.html?ws=47123"]);
});
