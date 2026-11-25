import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

import { installFakeBrowser } from "./helpers/fake_browser.js";

installFakeBrowser();
globalThis.document.body = {};

const { parseJoinLink, joinPagePath, formatTarget } = await import("../frontend/shared/js/join_link.ts");
const { joinSession, openManualJoin, lastAddress, foundRows, watchSessions } = await import("../frontend/main/js/pages/join_session.ts");
const { ProtocolMismatch } = await import("../frontend/shared/js/session_socket.ts");
const { mountAppBackLink } = await import("../frontend/session/js/claim.ts");

const memory = () => {
    const values = new Map();
    return { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
};

test("a join address is read with or without the scheme, and as a pasted page link", () => {
    const target = { host: "192.168.1.5", port: 47123 };
    assert.deepEqual(parseJoinLink("192.168.1.5:47123"), target);
    assert.deepEqual(parseJoinLink("  http://192.168.1.5:47123/  "), target);
    assert.deepEqual(parseJoinLink("http://192.168.1.5:47123/client.html"), target);
    assert.deepEqual(parseJoinLink("foh-laptop.local:47100"), { host: "foh-laptop.local", port: 47100 });
});

test("anything that is not a session address is refused", () => {
    for (const input of ["", "   ", "192.168.1.5", "https://192.168.1.5:47123", "ftp://host:21", "javascript:alert(1)",
        "http://user:pw@host:47123", "host :47123", "host:0", "host:99999"]) {
        assert.equal(parseJoinLink(input), null, JSON.stringify(input));
    }
});

test("the app opens its own crew page for the session", () => {
    assert.equal(joinPagePath({ host: "192.168.1.5", port: 47123 }), "session/client.html?host=192.168.1.5&port=47123");
    assert.equal(formatTarget({ host: "192.168.1.5", port: 47123 }), "192.168.1.5:47123");
});

test("joining opens the page only after the session answered", async () => {
    const opened = [];
    const probed = [];
    const problem = await joinSession("http://10.0.0.2:47150", { probe: async target => { probed.push(target); }, open: path => opened.push(path) });
    assert.equal(problem, null);
    assert.deepEqual(probed, [{ host: "10.0.0.2", port: 47150 }]);
    assert.deepEqual(opened, ["session/client.html?host=10.0.0.2&port=47150"]);
});

test("a bad address, a silent host or an incompatible version is explained and nothing opens", async () => {
    const deps = probe => ({ probe, open: () => assert.fail("must not open") });
    assert.match(await joinSession("nonsense", deps(async () => assert.fail("must not probe"))), /Enter the session's address/);
    assert.match(await joinSession("10.0.0.2:47150", deps(async () => { throw new Error("refused"); })), /No ShowFlow session answers/);
    const mismatch = new ProtocolMismatch();
    assert.equal(await joinSession("10.0.0.2:47150", deps(async () => { throw mismatch; })), mismatch.message);
});

test("typing an address is a fallback in a dialog, which remembers the last one that worked", async () => {
    const storage = memory();
    const opened = [];
    let answers = true;
    const deps = { probe: async () => { if (!answers) throw new Error("silent"); }, open: path => opened.push(path) };
    let dialog;
    globalThis.__dynamicPrompt = async options => { dialog = options; return null; };

    await openManualJoin(deps, storage);
    assert.equal(dialog.elements.find(e => e.id === "joinAddress").value, "");
    answers = false;
    assert.match((await dialog.onValidate({ joinAddress: "10.0.0.2:47150" })).joinAddress, /No ShowFlow session/);
    assert.equal(lastAddress(storage), "");
    answers = true;
    assert.equal(await dialog.onValidate({ joinAddress: "http://10.0.0.2:47150" }), null);
    assert.equal(opened.length, 1);
    await openManualJoin(deps, storage);
    assert.equal(dialog.elements.find(e => e.id === "joinAddress").value, "10.0.0.2:47150");
    storage.setItem("showflow.last_session_address", "junk");
    assert.equal(lastAddress(storage), "");
});

const found = (name, host = "192.168.1.20", compatible = true) => ({ host, port: 47111, session_id: `id-${name}`, show_name: name, compatible });

test("each found session can be joined; its name is only ever shown as text", () => {
    const joined = [];
    const hostile = "<img src=x onerror=alert(1)>";
    const rows = foundRows([found("Gala"), found(hostile), found("Old", "10.0.0.9", false)], session => joined.push(session.show_name));
    assert.ok(rows.every(item => item.html === undefined));
    assert.ok(rows.some(item => item.text === hostile) && rows.some(item => item.text === "192.168.1.20:47111"));
    const buttons = rows.filter(item => item.type === "button");
    assert.deepEqual(buttons.map(b => b.attributes.disabled), [undefined, undefined, "disabled"]);
    assert.ok(rows.some(item => /other ShowFlow version/.test(item.text ?? "")));
    buttons[0].events.click();
    assert.deepEqual(joined, ["Gala"]);
    const ids = rows.flatMap(item => (item.varId ? [item.varId] : []));
    assert.equal(new Set(ids).size, ids.length);
});

/** Runs the watcher with a hand-cranked timer. */
function watching(results) {
    const state = { alive: true, hidden: false, searches: 0, shown: [], problems: [], queued: [] };
    const next = () => new Promise(resolve => setImmediate(resolve));
    const started = watchSessions({
        find: async () => {
            const result = results[Math.min(state.searches++, results.length - 1)];
            if (result instanceof Error) throw result;
            return result;
        },
        show: list => state.shown.push(list.map(s => s.show_name)),
        problem: message => state.problems.push(message),
        alive: () => state.alive,
        hidden: () => state.hidden,
        schedule: callback => state.queued.push(callback)
    });
    state.tick = async () => { state.queued.shift()?.(); await next(); };
    state.started = started;
    return state;
}

test("the list keeps itself current, redrawing only when something changed", async () => {
    const watch = watching([[], [found("Gala")], [found("Gala")], new Error("no network"), []]);
    await watch.started;
    for (let i = 0; i < 4; i++) await watch.tick();
    assert.equal(watch.searches, 5);
    assert.deepEqual(watch.shown, [[], ["Gala"], []]);
    assert.deepEqual(watch.problems, [null, null, null, "no network", null]);
    assert.equal(watch.queued.length, 1, "one search at a time");
});

test("searching pauses while nobody looks and stops when the list is gone", async () => {
    const watch = watching([[]]);
    await watch.started;
    watch.hidden = true;
    await watch.tick();
    assert.equal(watch.searches, 1);
    watch.hidden = false;
    await watch.tick();
    assert.equal(watch.searches, 2);
    watch.alive = false;
    await watch.tick();
    assert.deepEqual([watch.searches, watch.queued.length], [2, 0]);
});

test("only pages the app opened for another host get a link back to the show list", () => {
    const created = [];
    globalThis.__dynamicGenerator = definition => { created.push(definition); return {}; };
    assert.equal(mountAppBackLink("?ws=47123"), false);
    assert.equal(mountAppBackLink("?host=10.0.0.2&port=47150"), true);
    assert.equal(created.length, 1);
    assert.equal(created[0].attributes.href, "../index.html");
});
