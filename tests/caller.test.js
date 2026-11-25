import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";
import { installFakeBrowser } from "./helpers/fake_browser.js";

const { storage } = installFakeBrowser();
const { askClaim, appAccountClaim, AccessFlow } = await import("../frontend/session/js/claim.ts");
const { codeProblem } = await import("../frontend/shared/js/caller_code.ts");
const { resolveSessionAddress, ProtocolMismatch } = await import("../frontend/shared/js/session_socket.ts");

const policy = (open = false, code = false, request = false, account = false) => ({ open, code, request, account });

/** A prompt that records what was asked and answers with `answer` (null: cancelled). */
function prompting(answer) {
    const asked = [];
    return { asked, prompt: async options => { asked.push(options); return answer; } };
}
const ids = options => options.elements.map(element => element.id).filter(Boolean);

test("open access passes straight through: nothing is asked", async () => {
    const { asked, prompt } = prompting({});
    assert.deepEqual(await askClaim("call", policy(true), "Ann", prompt), { method: "open", name: "Ann" });
    assert.equal(asked.length, 0);
});

test("with a code, the code is asked first, in a hidden field with no autocorrect", async () => {
    for (const [kind, title, label] of [["call", "Become show caller", "Caller code"], ["view", "Join the session", "Access code"]]) {
        const { asked, prompt } = prompting({ accessName: " Sam ", accessCode: "P@ssw0rd!" });
        assert.deepEqual(await askClaim(kind, policy(false, true), "Sam", prompt), { method: "code", name: "Sam", code: "P@ssw0rd!" });
        const [options] = asked;
        assert.equal(options.title, title);
        assert.deepEqual(ids(options), ["accessName", "accessCode"]);
        const field = options.elements.find(element => element.id === "accessCode");
        assert.deepEqual([field.label, field.inputType], [label, "password"]);
        assert.deepEqual([field.attributes.autocapitalize, field.attributes.autocorrect, field.attributes.spellcheck, field.attributes.maxlength], ["off", "off", "false", 32]);
        assert.ok(options.elements.some(element => element.text === "Show code"));
        assert.equal(options.elements.find(element => element.id === "accessName").value, "Sam", "a known name is filled in");
        assert.deepEqual(options.onValidate({ accessName: "x", accessCode: "abc123" }), {});
        assert.ok(options.onValidate({ accessName: "x", accessCode: "abc12" }).accessCode);
        assert.ok(options.onValidate({ accessName: " ", accessCode: "abc123" }).accessName);
        assert.equal(await askClaim(kind, policy(false, true), "Sam", prompting(null).prompt), null, "cancelled");
    }
});

test("a request asks only for a name", async () => {
    const request = prompting({ accessName: "Kim" });
    assert.deepEqual(await askClaim("view", policy(false, false, true), "", request.prompt), { method: "request", name: "Kim" });
    assert.deepEqual(ids(request.asked[0]), ["accessName"]);
    assert.deepEqual(request.asked[0].onValidate({ accessName: "Kim" }), {});
});

test("a device with no way in is told why instead of being asked for anything", async () => {
    const byAccount = policy(false, false, false, true);
    for (const [kind, closed, inApp, expected] of [
        ["call", policy(), false, /not opened the caller role/], ["view", policy(), true, /not opened this session/],
        ["call", byAccount, false, /owner made a caller/], ["call", byAccount, true, /no caller pass/],
        ["view", byAccount, false, /people with access/], ["view", byAccount, true, /needs access to the show/]
    ]) {
        const { asked, prompt } = prompting({});
        assert.equal(await askClaim(kind, closed, "x", prompt, inApp), null);
        assert.match(asked[0].elements[0].text, expected);
    }
});

const welcome = (caller, view = policy(true)) => ({ session_id: "abc", show_id: "show-1", nonce: "n-1", policy: caller, view_policy: view });

function flow({ ask, account, kind = "call", allowed = policy(false, false, true) } = {}) {
    const log = [];
    const instance = new AccessFlow({
        kind,
        send: claim => log.push(["send", claim]),
        onGranted: () => log.push(["granted"]),
        onLost: reason => log.push(["lost", reason]),
        notify: text => log.push(["notify", text]),
        ask, account
    });
    instance.welcome(kind === "call" ? welcome(allowed) : welcome(policy(), allowed));
    return { instance, log };
}
const status = (state, reason = null, resume_token = null) => ({ state, reason, resume_token });

test("each gate follows its own rules and keeps its own token", () => {
    storage.clear();
    const call = flow({ allowed: policy(false, true) });
    const view = flow({ kind: "view", allowed: policy(false, false, true) });
    assert.ok(call.instance.policy.code && view.instance.policy.request);
    call.instance.adopt({ policy: policy(true), view_policy: policy(false, true) });
    view.instance.adopt({ policy: policy(true), view_policy: policy(false, true) });
    assert.ok(call.instance.policy.open && view.instance.policy.code);

    call.instance.handle(status("granted", null, "call-token"));
    view.instance.handle(status("granted", null, "view-token"));
    assert.deepEqual([storage.get("showflow.caller.abc"), storage.get("showflow.view.abc")], ["call-token", "view-token"]);
    view.instance.handle(status("revoked", "The host removed you"));
    assert.deepEqual([storage.get("showflow.caller.abc"), storage.get("showflow.view.abc")], ["call-token", undefined]);
});

test("a grant stores the resume token", () => {
    storage.clear();
    const { instance, log } = flow();
    instance.handle(status("granted", null, "tok-1"));
    assert.equal(storage.get("showflow.caller.abc"), "tok-1");
    assert.deepEqual(log, [["granted"]]);
    assert.ok(instance.granted);
});

test("waiting and wrong codes are reported without losing a stored token", () => {
    storage.set("showflow.caller.abc", "old");
    const { instance, log } = flow();
    instance.claim({ method: "request", name: "x" });
    instance.handle(status("pending"));
    instance.claim({ method: "code", name: "x", code: "000000" });
    instance.handle(status("denied", "Wrong code"));
    assert.deepEqual(log.filter(entry => entry[0] === "notify").map(entry => entry[1]), ["Waiting for the host to approve…", "Wrong code"]);
    assert.equal(storage.get("showflow.caller.abc"), "old");
});

test("a stale stored token is forgotten when resuming fails, and revocation forgets it too", () => {
    storage.set("showflow.caller.abc", "stale");
    const { instance, log } = flow();
    assert.ok(instance.resume());
    assert.deepEqual(log[0], ["send", { method: "resume", token: "stale" }]);
    instance.handle(status("denied", "Your access has ended"));
    assert.deepEqual(log[1], ["lost", "Your access has ended"]);
    assert.equal(storage.get("showflow.caller.abc"), undefined);
    assert.equal(instance.resume(), false);

    instance.handle(status("granted", null, "tok"));
    instance.handle(status("revoked", "The host removed your caller access"));
    assert.equal(instance.granted, false);
    assert.equal(storage.get("showflow.caller.abc"), undefined);
    assert.deepEqual(log.at(-1), ["lost", "The host removed your caller access"]);
});

test("the button sends whatever the policy calls for, and nothing when cancelled", async () => {
    const asked = [];
    const { instance, log } = flow({ ask: async (kind, current, name) => { asked.push([kind, current.request, name]); return name ? { method: "request", name } : null; } });
    await instance.request("");
    assert.deepEqual(log, []);
    await instance.request("Kim");
    assert.deepEqual(log, [["send", { method: "request", name: "Kim" }]]);
    assert.deepEqual(asked, [["call", true, ""], ["call", true, "Kim"]]);
});

test("a pass is used without asking anything; refused, it is not offered again", async () => {
    const challenges = [];
    const pass = { method: "account", pass: "p", proof: "q" };
    let asked = 0;
    const { instance, log } = flow({
        kind: "view",
        allowed: policy(false, false, false, true),
        account: async challenge => { challenges.push([challenge.show_id, challenge.session_id, challenge.nonce]); return pass; },
        ask: async () => { asked++; return null; }
    });
    await instance.request("");
    assert.deepEqual(log, [["send", pass]]);
    assert.deepEqual(challenges, [["show-1", "abc", "n-1"]]);
    instance.handle(status("denied", "You are not a member of this show"));
    assert.deepEqual(log.at(-1), ["notify", "You are not a member of this show"]);
    await instance.request("");
    assert.deepEqual([challenges.length, asked], [1, 1]);
});

test("without account access no pass is looked for", async () => {
    const off = flow({ account: async () => assert.fail("account access is off"), ask: async () => ({ method: "request", name: "Kim" }) });
    await off.instance.request("Kim");
    assert.equal(off.log.length, 1);
});

test("only the app has a pass to offer, and it signs this connection's challenge", async () => {
    const challenge = { session_id: "s", show_id: "show-1", nonce: "n" };
    const calls = [];
    globalThis.__invoke = async (command, args) => { calls.push([command, args]); return { pass: "p", proof: "q" }; };
    assert.equal(await appAccountClaim(challenge), null, "a browser");
    assert.deepEqual(calls, []);
    window.__TAURI_INTERNALS__ = {};
    try {
        assert.deepEqual(await appAccountClaim(challenge), { method: "account", pass: "p", proof: "q" });
        assert.deepEqual(calls, [["caller_claim", { showId: "show-1", sessionId: "s", nonce: "n" }]]);
        globalThis.__invoke = async () => null;
        assert.equal(await appAccountClaim(challenge), null, "no pass for this show");
    } finally {
        delete window.__TAURI_INTERNALS__;
        delete globalThis.__invoke;
    }
});

test("the code rules match the server's", () => {
    assert.match(codeProblem("12345"), /at least 6/);
    assert.equal(codeProblem("123456"), null);
    assert.equal(codeProblem("a".repeat(32)), null);
    assert.match(codeProblem("a".repeat(33)), /at most 32/);
    for (const ok of ["3FA9C1D27B", "P@ss!w#rd", "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~", "abcdef"]) assert.equal(codeProblem(ok), null, ok);
    for (const bad of ["abc def", "abcdef ", "abcdéf", "abc\tdef"]) assert.match(codeProblem(bad), /without spaces/, JSON.stringify(bad));
});

test("the socket address comes from the app, from ?ws=, or from the session's own web server", async () => {
    const at = (search, hostname = "192.168.1.5") => ({ hostname, search });
    const none = () => { throw new Error("no request expected"); };
    assert.deepEqual(await resolveSessionAddress(at(""), { host: "127.0.0.1", ws: 47111, isHost: true, sessionId: "s" }, none), { host: "127.0.0.1", wsPort: 47111 });
    assert.deepEqual(await resolveSessionAddress(at("?ws=47150"), undefined, none), { host: "192.168.1.5", wsPort: 47150 });

    const urls = [];
    const info = (body, ok = true) => async url => { urls.push(url); return { ok, json: async () => body }; };
    assert.deepEqual(await resolveSessionAddress(at(""), undefined, info({ protocol: 2, ws_port: 47102 })), { host: "192.168.1.5", wsPort: 47102 });
    // The ShowFlow app asking a session running on another machine.
    assert.deepEqual(await resolveSessionAddress(at("?host=10.0.0.7&port=47190", "tauri.localhost"), undefined, info({ protocol: 2, ws_port: 47191 })), { host: "10.0.0.7", wsPort: 47191 });
    assert.deepEqual(urls, ["/session-info", "http://10.0.0.7:47190/session-info"]);

    await assert.rejects(resolveSessionAddress(at(""), undefined, info({ protocol: 3, ws_port: 1 })), ProtocolMismatch);
    await assert.rejects(resolveSessionAddress(at(""), undefined, info({}, false)), /no running ShowFlow session/);
});
