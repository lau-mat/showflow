import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const hints = new Map();
globalThis.document = { getElementById: id => hints.get(id) ?? hints.set(id, {}).get(id), activeElement: null };

const { installApi } = await import("../frontend/main/js/api/index.ts");
const panel = await import("../frontend/main/js/pages/session_panel.ts");
const access = await import("../frontend/main/js/pages/session_settings.ts");

const show = { id: "show-1", name: "Gala" };
const policy = mode => ({ open: mode === "open", code: mode === "code", request: mode === "request", account: mode === "account" });
const settings = (view = "open", caller = "request", extra = {}) => ({ view: policy(view), view_code: null, caller: policy(caller), caller_code: null, ...extra });

function hostApi(overrides = {}) {
    const calls = [];
    const record = name => async (...args) => { calls.push([name, ...args]); };
    const host = {
        calls,
        settings: async () => settings(),
        startSession: async (...args) => { calls.push(["startSession", ...args]); return { session_id: "s1" }; },
        stopSession: record("stopSession"), saveSettings: record("saveSettings"), setCode: record("setCode"),
        answerRequest: record("answerRequest"), removeDevice: record("removeDevice"),
        revokeCaller: record("revokeCaller"), openCallerWindow: record("openCallerWindow"),
        ...overrides
    };
    installApi({ host });
    return host;
}

/** Runs a dialog like a user pressing the confirm button with these values. */
function answering(values, seen = []) {
    globalThis.__dynamicPrompt = async options => {
        seen.push(options);
        const errors = options.onValidate ? await options.onValidate(values) : null;
        return errors && Object.keys(errors).length ? null : values;
    };
    return seen;
}

const options = select => select.options.map(o => o.attributes.value);
const selected = select => select.options.filter(o => o.attributes.selected).map(o => o.attributes.value);
const byId = elements => Object.fromEntries(elements.filter(e => e.id).map(e => [e.id, e]));

test("starting asks nothing", async () => {
    const host = hostApi();
    globalThis.__dynamicPrompt = async () => assert.fail("no dialog");
    assert.equal(await panel.startShowSession(show), "s1");
    assert.deepEqual(host.calls, [["startSession", "show-1"]]);
});

test("a start that fails says why", async () => {
    hostApi({ startSession: async () => { throw "No two free ports between 47100 and 47199."; } });
    const seen = answering({});
    assert.equal(await panel.startShowSession(show), null);
    assert.match(seen[0].elements[0].text, /No two free ports/);
});

test("each gate is one dropdown with the same choices; joining defaults to open, calling to approval", async () => {
    hostApi();
    const seen = answering(null);
    globalThis.__dynamicPrompt = async dialog => { seen.push(dialog); return null; };
    await access.openSessionSettings(show);
    const fields = byId(seen[0].elements);
    assert.deepEqual([fields["settings-view"].type, fields["settings-call"].type], ["select-label", "select-label"]);
    assert.deepEqual([...options(fields["settings-view"])].sort(), [...options(fields["settings-call"])].sort());
    assert.deepEqual([options(fields["settings-view"])[0], options(fields["settings-call"])[0]], ["open", "request"], "the default is listed first");
    assert.deepEqual([selected(fields["settings-view"]), selected(fields["settings-call"])], [["open"], ["request"]]);
    assert.deepEqual([fields["settings-view-code"].style, fields["settings-call-code"].style], [{ display: "none" }, { display: "none" }]);
    assert.equal(fields["settings-call-hint"].text, access.GATES.call.hints.request);
});

test("a choice and a policy mean the same thing both ways", () => {
    for (const mode of ["open", "request", "code", "account", "host"]) assert.equal(access.modeOf(access.policyFor(mode)), mode);
    assert.deepEqual(access.policyFor("host"), { open: false, code: false, request: false, account: false });
});

test("saving keeps each gate's code apart, generates empty ones and remembers unused ones", async () => {
    const host = hostApi({ settings: async () => settings("open", "code", { caller_code: "Caller-1", view_code: "Door-123" }) });
    answering({ "settings-view": "code", "settings-view-code": "", "settings-call": "account", "settings-call-code": "ignored1" });
    await access.openSessionSettings(show);
    assert.deepEqual(host.calls, [["saveSettings", "show-1", settings("code", "account", { view_code: null, caller_code: "Caller-1" })]]);
});

test("a code that breaks the rules, or a failing save, keeps the dialog open", async () => {
    const host = hostApi();
    const seen = answering({ "settings-view": "code", "settings-view-code": "short", "settings-call": "code", "settings-call-code": "with space" });
    await access.openSessionSettings(show);
    assert.equal(host.calls.length, 0);
    const errors = await seen[0].onValidate({ "settings-view": "code", "settings-view-code": "short", "settings-call": "code", "settings-call-code": "with space" });
    assert.match(errors["settings-view-code"], /at least 6/);
    assert.match(errors["settings-call-code"], /without spaces/);
    // A code in a field that is not in use does not matter.
    assert.equal(await seen[0].onValidate({ "settings-view": "open", "settings-view-code": "x", "settings-call": "request" }), null);
    hostApi({ saveSettings: async () => { throw new Error("disk full"); } });
    const failing = answering({ "settings-view": "open", "settings-call": "request" });
    await access.openSessionSettings(show);
    assert.deepEqual(await failing[0].onValidate({ "settings-view": "open", "settings-call": "request" }), { "settings-view": "disk full" });
});

test("closing the door to others warns that nobody else can call either", () => {
    assert.match(access.accessWarning("host", "request"), /nobody else can call/);
    assert.equal(access.accessWarning("host", "host"), "");
    assert.equal(access.accessWarning("open", "request"), "");
});

const status = (view = {}, extra = {}) => ({
    session_id: "s1", show_id: "show-1", show_name: "Gala", url: "http://192.168.1.5:47123", account_ready: true, discoverable: true, ...extra,
    view: {
        settings: settings("code", "code", { view_code: "DOOR1234", caller_code: "ABCDEF12" }),
        view_paused_seconds: null, call_paused_seconds: null, current_cue: null, people: [], callers: [], requests: [], ...view
    }
});
const actions = log => ({ refresh: async () => { log.push("refresh"); }, close: () => log.push("close"), toggleCode: kind => log.push(["toggle", kind]) });
const content = (state, shown = [], log = []) => panel.panelContent(state, "<svg/>", new Set(shown), actions(log));
const texts = defs => defs.map(d => d.text).filter(t => typeof t === "string");
const buttons = (defs, text) => defs.filter(d => d.type === "button" && d.text === text);
const person = (client_id, name, extra = {}) => ({ client_id, name, role_id: null, admitted: true, is_caller: false, is_host: false, ...extra });

test("the panel shows one join address and hides both codes until asked", () => {
    hostApi();
    const hidden = content(status({ people: [person("a", "Ann")] }));
    assert.equal(hidden.filter(d => d.type === "svg" && d.sketch).length, 1, "exactly one QR code");
    assert.equal(hidden.find(d => d.id === "sessionUrl").attributes.value, "http://192.168.1.5:47123");
    assert.ok(!texts(hidden).includes("ABCDEF12") && !texts(hidden).includes("DOOR1234"));
    assert.ok(texts(content(status(), ["view"])).includes("DOOR1234"));
    assert.ok(!texts(content(status(), ["view"])).includes("ABCDEF12"), "revealed one at a time");
    assert.ok(texts(hidden).includes("1 device connected"));
});

test("requests to join and to call are answered separately; devices and callers can be removed", async () => {
    const host = hostApi();
    const view = {
        settings: settings("request", "request"),
        requests: [{ client_id: "c1", name: "Kim", kind: "view" }, { client_id: "c2", name: "Lee", kind: "call" }],
        callers: [{ grant_id: "g1", name: "Host", method: "host", online: true }, { grant_id: "g2", name: "Sam", method: "code", online: false }],
        people: [person("h", "Host", { is_caller: true, is_host: true }), person("d1", null), person("w", "Waiting", { admitted: false })]
    };
    const defs = content(status(view));
    assert.ok(texts(defs).includes("wants to join") && texts(defs).includes("wants to call") && texts(defs).includes("code · offline"));
    assert.ok(texts(defs).includes("Unnamed device") && !texts(defs).includes("Waiting"));
    const trash = defs.filter(d => d.file === "icons/delete.svg");
    assert.deepEqual(trash.map(d => d.attributes.title), ["Remove from the session", "Remove caller access"], "never the host itself");

    await buttons(defs, "Approve")[0].events.click();
    await buttons(defs, "Deny")[1].events.click();
    for (const icon of trash) await icon.events.click();
    assert.deepEqual(host.calls, [
        ["answerRequest", "s1", "c1", "view", true], ["answerRequest", "s1", "c2", "call", false],
        ["removeDevice", "s1", "d1"], ["revokeCaller", "s1", "g2"]
    ]);
});

test("changing a dropdown saves just that gate", async () => {
    const host = hostApi();
    const defs = content(status());
    await byId(defs)["panel-view"].events.change({ target: { value: "request", blur() {} } });
    assert.equal(hints.get("panel-view-hint").textContent, access.GATES.view.hints.request, "the line under it follows");
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(host.calls, [["saveSettings", "show-1", settings("request", "code", { view_code: "DOOR1234", caller_code: "ABCDEF12" })]]);
});

test("while joining is open there is nothing to remove, and there are no code rows without codes", () => {
    hostApi();
    const open = content(status({ settings: settings("open", "request"), people: [person("a", "Ann")] }), ["view", "call"]);
    assert.ok(!texts(open).includes("Joined devices") && buttons(open, "Generate").length === 0);
});

test("the panel says when this computer cannot be found, or cannot check passes", () => {
    hostApi();
    const says = (state, pattern) => texts(content(state)).some(t => pattern.test(t));
    assert.ok(says(status({}, { discoverable: false }), /cannot find this session/));
    assert.ok(!says(status(), /cannot find this session/));
    for (const gate of [settings("account", "request"), settings("open", "account")]) {
        assert.ok(says(status({ settings: gate }, { account_ready: false }), /has to sync with the cloud/));
        assert.ok(!says(status({ settings: gate }), /has to sync with the cloud/));
    }
    const paused = content(status({ view_paused_seconds: 42 }));
    assert.ok(texts(paused).some(t => /paused for 42 s/.test(t)));
});

test("ending a session asks first", async () => {
    const host = hostApi();
    const log = [];
    const end = () => buttons(content(status(), [], log), "End session")[0].events.click();
    globalThis.__dynamicPrompt = async () => null;
    await end();
    assert.deepEqual(host.calls, []);
    globalThis.__dynamicPrompt = async () => ({});
    await end();
    assert.deepEqual(host.calls, [["stopSession", "s1"]]);
    assert.ok(log.includes("close"));
});

test("changing and generating a code go to the right gate with valid values only", async () => {
    const host = hostApi();
    // The join gate's buttons come first, then the caller gate's.
    let seen = answering({ newCode: "N3w-C0de!" });
    await buttons(content(status()), "Change")[0].events.click();
    assert.deepEqual(host.calls.at(-1), ["setCode", "s1", "view", "N3w-C0de!"]);
    assert.ok((await seen[0].onValidate({ newCode: "abc" })).newCode);
    seen = answering({ codeLength: "32" });
    await buttons(content(status()), "Generate")[1].events.click();
    assert.deepEqual(host.calls.at(-1), ["setCode", "s1", "call", null, 32]);
    for (const bad of ["5", "33", "7.5", ""]) assert.ok((await seen[0].onValidate({ codeLength: bad })).codeLength, bad);
});
