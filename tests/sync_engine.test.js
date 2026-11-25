import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const { SyncEngine } = await import("../frontend/main/js/sync/engine.ts");
const { ChangeBus } = await import("../frontend/main/js/sync/bus.ts");
const { SyncStatusStore } = await import("../frontend/main/js/sync/status.ts");
const { ApiError, NetworkError } = await import("../frontend/main/js/api/types.ts");

const op = (id, entity = "line") => ({ op_id: id, entity, id: `${id}-entity`, kind: "set", fields: {} });

/** A local store with the real semantics of the app database's outbox and cursors. */
function fakeLocal({ pending = [], shows = [] } = {}) {
    const log = [];
    const store = {
        outbox: [...pending],
        shows: new Map(shows.map(show => [show.show_id, { synced: true, role: "owner", server_rev: 0, ...show }])),
        log,
        applied: [],
        async state() {
            return { account: "alice", shows: [...store.shows.values()], pending_total: store.outbox.length };
        },
        async pending(limit) { return store.outbox.slice(0, limit); },
        async ack(showId, opIds) {
            log.push(["ack", showId, opIds]);
            store.outbox = store.outbox.filter(item => !opIds.includes(item.op.op_id));
            if (!store.shows.has(showId)) store.shows.set(showId, { show_id: showId, server_rev: 0, synced: true, role: "owner" });
        },
        async applyChanges(showId, changes, rev, role) {
            log.push(["apply", showId, changes.map(change => change.rev), rev, role]);
            const have = store.shows.get(showId);
            store.shows.set(showId, { show_id: showId, synced: true, role: role ?? "owner", server_rev: Math.max(have?.server_rev ?? 0, rev) });
            store.applied.push(...changes);
        },
        async forgetShow(showId) {
            log.push(["forget", showId]);
            store.shows.delete(showId);
            store.outbox = store.outbox.filter(item => item.show_id !== showId);
        },
        async adoptAccount(userId) { log.push(["adopt", userId]); return 0; }
    };
    return store;
}

function setup({ local = fakeLocal(), routes = {}, state = "authenticated" } = {}) {
    const requests = [];
    const bus = new ChangeBus();
    const status = new SyncStatusStore();
    const events = [];
    bus.subscribe(change => events.push(change.showId));
    const rest = {
        async request(path, options = {}) {
            requests.push([path, options.body]);
            const handler = Object.entries(routes).find(([prefix]) => path.startsWith(prefix))?.[1];
            if (!handler) throw new Error(`unexpected request ${path}`);
            return handler(path, options.body);
        }
    };
    const timers = [];
    const engine = new SyncEngine({
        session: { state, user: { id: "alice" }, subscribe: () => () => {} },
        rest, local, bus, status, now: () => 1234,
        setTimer: (callback, ms) => { timers.push([callback, ms]); return timers.length; },
        clearTimer: () => {}
    });
    return { engine, local, requests, status, events, timers, bus };
}

const applied = ops => ops.map((o, i) => ({ status: "applied", op_id: o.op_id, rev: i + 1 }));

test("queued edits are pushed oldest first, one show at a time, then acknowledged", async () => {
    const local = fakeLocal({
        pending: [
            { show_id: "A", op: op("a1") }, { show_id: "A", op: op("a2") },
            { show_id: "B", op: op("b1") }, { show_id: "A", op: op("a3") }
        ],
        shows: [{ show_id: "A" }, { show_id: "B" }]
    });
    const { engine, requests, status } = setup({
        local,
        routes: {
            "shows/A/ops": (_p, body) => ({ results: applied(body.ops), rev: 9 }),
            "shows/B/ops": (_p, body) => ({ results: applied(body.ops), rev: 9 }),
            "shows?": () => [], "shows": () => []
        }
    });
    await engine.syncNow();
    const pushes = requests.filter(([path]) => path.endsWith("/ops")).map(([path, body]) => [path, body.ops.map(o => o.op_id)]);
    assert.deepEqual(pushes, [["shows/A/ops", ["a1", "a2"]], ["shows/B/ops", ["b1"]], ["shows/A/ops", ["a3"]]]);
    assert.equal(local.outbox.length, 0);
    assert.deepEqual(status.status, { phase: "idle", pending: 0, lastSyncedAt: 1234, message: null });
    assert.equal(local.log[0][0], "adopt", "legacy shows are claimed before anything is pushed");
});

test("a retry after a lost response is harmless: duplicates are acknowledged like applied ops", async () => {
    const local = fakeLocal({ pending: [{ show_id: "A", op: op("a1") }], shows: [{ show_id: "A" }] });
    const { engine } = setup({
        local,
        routes: {
            "shows/A/ops": (_p, body) => ({ results: body.ops.map(o => ({ status: "duplicate", op_id: o.op_id, rev: 3 })), rev: 3 }),
            "shows": () => []
        }
    });
    await engine.syncNow();
    assert.equal(local.outbox.length, 0);
});

test("pulling fetches only shows that moved on, resuming from the stored revision", async () => {
    const local = fakeLocal({ shows: [{ show_id: "A", server_rev: 5 }, { show_id: "B", server_rev: 2 }] });
    const change = rev => ({ rev, op_id: `c${rev}`, entity: "role", id: "r", kind: "set", fields: {}, author_id: null });
    const { engine, requests, events } = setup({
        local,
        routes: {
            "shows/B/changes": () => ({ changes: [change(3), change(4)], rev: 4, more: false, deleted: false }),
            "shows": () => [
                { id: "A", name: "a", time: 1, role: "owner", rev: 5 },
                { id: "B", name: "b", time: 1, role: "owner", rev: 4 }
            ]
        }
    });
    await engine.syncNow();
    assert.deepEqual(requests.map(r => r[0]), ["shows", "shows/B/changes?since=2"]);
    assert.deepEqual(local.log.filter(l => l[0] === "apply"), [["apply", "B", [3, 4], 4, "owner"]]);
    assert.deepEqual(events, ["B"], "pages watching B are told, nothing else");
});

test("long histories are pulled page by page and the cursor follows the last change", async () => {
    const local = fakeLocal();
    const change = rev => ({ rev, op_id: `c${rev}`, entity: "role", id: "r", kind: "set", fields: {}, author_id: null });
    const pages = [
        { changes: [change(1), change(2)], rev: 5, more: true, deleted: false },
        { changes: [change(3), change(4), change(5)], rev: 5, more: false, deleted: false }
    ];
    const { engine, requests, events } = setup({
        local,
        routes: {
            "shows/N/changes": () => pages.shift(),
            "shows": () => [{ id: "N", name: "n", time: 1, role: "editor", rev: 5 }]
        }
    });
    await engine.syncNow();
    assert.deepEqual(requests.map(r => r[0]), ["shows", "shows/N/changes?since=0", "shows/N/changes?since=2"]);
    assert.deepEqual(local.log.filter(l => l[0] === "apply"), [["apply", "N", [1, 2], 2, "editor"], ["apply", "N", [3, 4, 5], 5, "editor"]]);
    assert.deepEqual(events, ["N", null], "a new show also changes the list");
});

test("a role change is applied even when no content changed", async () => {
    const local = fakeLocal({ shows: [{ show_id: "A", server_rev: 5, role: "editor" }] });
    const { engine, requests } = setup({
        local,
        routes: {
            "shows/A/changes": () => ({ changes: [], rev: 5, more: false, deleted: false }),
            "shows": () => [{ id: "A", name: "a", time: 1, role: "viewer", rev: 5 }]
        }
    });
    await engine.syncNow();
    assert.ok(requests.some(r => r[0].startsWith("shows/A/changes")));
    assert.equal(local.shows.get("A").role, "viewer");
});

test("shows that left the cloud's list are forgotten, but never-synced local shows are kept", async () => {
    const local = fakeLocal({ shows: [{ show_id: "gone", server_rev: 4 }, { show_id: "mine", server_rev: 0, synced: false }] });
    const { engine, events } = setup({ local, routes: { "shows": () => [] } });
    await engine.syncNow();
    assert.ok(local.shows.has("mine"));
    assert.ok(!local.shows.has("gone"));
    assert.deepEqual(events, ["gone", null]);
});

test("an op the server refuses makes the engine re-download the show and say why", async () => {
    const local = fakeLocal({ pending: [{ show_id: "A", op: op("a1") }, { show_id: "A", op: op("a2") }], shows: [{ show_id: "A" }] });
    const { engine, status, events } = setup({
        local,
        routes: {
            "shows/A/ops": (_p, body) => ({
                results: [{ status: "rejected", op_id: body.ops[0].op_id, error: "You may not change this show" }, ...applied(body.ops.slice(1))],
                rev: 4
            }),
            "shows": () => []
        }
    });
    await engine.syncNow();
    assert.ok(local.log.some(l => l[0] === "forget" && l[1] === "A"));
    assert.equal(local.outbox.length, 0);
    assert.match(status.status.message, /not accepted: You may not change this show/);
    assert.ok(events.includes(null));
});

test("losing access to a show drops it instead of retrying forever", async () => {
    for (const code of [403, 404, 410]) {
        const local = fakeLocal({ pending: [{ show_id: "A", op: op("a1") }, { show_id: "B", op: op("b1") }], shows: [{ show_id: "A" }, { show_id: "B" }] });
        const { engine, status } = setup({
            local,
            routes: {
                "shows/A/ops": () => { throw new ApiError(code, "nope"); },
                "shows/B/ops": (_p, body) => ({ results: applied(body.ops), rev: 1 }),
                "shows": () => [{ id: "B", name: "b", time: 1, role: "owner", rev: 0 }]
            }
        });
        await engine.syncNow();
        assert.ok(!local.shows.has("A"), String(code));
        assert.equal(local.outbox.length, 0, "the other show was still pushed");
        assert.match(status.status.message, /no longer available/);
    }
});

test("offline keeps the edits queued and says so; other failures are reported", async () => {
    const local = fakeLocal({ pending: [{ show_id: "A", op: op("a1") }], shows: [{ show_id: "A" }] });
    const offline = setup({ local, routes: { "shows/A/ops": () => { throw new NetworkError(); } } });
    await offline.engine.syncNow();
    assert.equal(local.outbox.length, 1);
    assert.deepEqual([offline.status.status.phase, offline.status.status.pending], ["offline", 1]);

    const broken = setup({ local, routes: { "shows/A/ops": () => { throw new ApiError(500, "boom"); } } });
    await broken.engine.syncNow();
    assert.deepEqual([broken.status.status.phase, broken.status.status.message], ["error", "boom"]);
    assert.equal(local.outbox.length, 1, "still queued for the next attempt");
});

test("nothing is sent without a signed-in, online session", async () => {
    for (const state of ["unauthenticated", "offlineGrace", "graceExpired"]) {
        const local = fakeLocal({ pending: [{ show_id: "A", op: op("a1") }], shows: [{ show_id: "A" }] });
        const { engine, requests, status } = setup({ local, state, routes: {} });
        await engine.syncNow();
        assert.deepEqual(requests, [], state);
        assert.equal(status.status.phase, "paused");
        assert.equal(local.outbox.length, 1);
    }
});

test("overlapping triggers never run two cycles at once, and an edit made mid-sync is not missed", async () => {
    const local = fakeLocal({ shows: [{ show_id: "A" }] });
    let inFlight = 0, maxInFlight = 0, indexCalls = 0;
    let release;
    const gate = new Promise(resolve => { release = resolve; });
    const { engine } = setup({
        local,
        routes: {
            "shows": async () => {
                indexCalls++;
                inFlight++; maxInFlight = Math.max(maxInFlight, inFlight);
                if (indexCalls === 1) await gate;
                inFlight--;
                return [{ id: "A", name: "a", time: 1, role: "owner", rev: 0 }];
            }
        }
    });
    const first = engine.syncNow();
    await new Promise(resolve => setImmediate(resolve));
    const second = engine.syncNow();   // arrives while the first is still pulling
    const third = engine.syncNow();
    release();
    await Promise.all([first, second, third]);
    assert.equal(maxInFlight, 1);
    assert.equal(indexCalls, 2, "one rerun covers every trigger that arrived meanwhile");
});

test("local edits schedule a debounced sync", () => {
    const { engine, bus, timers } = setup({ routes: {} });
    engine.start();
    const before = timers.length;
    bus.emitLocalWrite();
    bus.emitLocalWrite();
    const scheduled = timers.slice(before);
    assert.ok(scheduled.length >= 1);
    assert.ok(scheduled.every(([, ms]) => ms === 800), "waits for the user to pause");
});
