import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const { parseSse, toLiveEvent, LiveUpdates } = await import("../frontend/main/js/sync/live.ts");
const { ApiError, NetworkError } = await import("../frontend/main/js/api/types.ts");
const { ChangeBus } = await import("../frontend/main/js/sync/bus.ts");

const encoder = new TextEncoder();
function streamOf(chunks) {
    return new ReadableStream({
        start(controller) { for (const chunk of chunks) controller.enqueue(typeof chunk === "string" ? encoder.encode(chunk) : chunk); controller.close(); }
    });
}
async function collect(chunks) {
    const out = [];
    for await (const message of parseSse(streamOf(chunks))) out.push(message);
    return out;
}

test("server-sent events are split into messages whatever the chunk boundaries", async () => {
    const wire = "event: ready\ndata: {}\n\n: ping\n\nevent: show\ndata: {\"show_id\":\"a\",\"rev\":3}\n\n";
    const expected = [{ event: "ready", data: "{}" }, { event: "show", data: '{"show_id":"a","rev":3}' }];
    assert.deepEqual(await collect([wire]), expected);
    // One byte at a time, including the blank line that ends a message.
    assert.deepEqual(await collect([...wire].map(char => char)), expected);
    // CRLF line endings, and a multi-byte character cut in half between chunks.
    assert.deepEqual(await collect(["event: show\r\ndata: x\r\n\r\n"]), [{ event: "show", data: "x" }]);
    const bytes = encoder.encode("event: show\ndata: café\n\n");
    assert.deepEqual(await collect([bytes.slice(0, 20), bytes.slice(20)]), [{ event: "show", data: "café" }]);
    // A message that never ended is not delivered half-way.
    assert.deepEqual(await collect(["event: show\ndata: partial"]), []);
});

test("multi-line data is joined and keep-alive comments are ignored", async () => {
    assert.deepEqual(await collect([": ping\n\n", "data: a\ndata: b\n\n"]), [{ event: "message", data: "a\nb" }]);
});

test("stream messages become typed events; unknown or broken ones are dropped", () => {
    assert.deepEqual(toLiveEvent({ event: "ready", data: "{}" }), { type: "ready" });
    assert.deepEqual(toLiveEvent({ event: "access", data: "{}" }), { type: "access" });
    assert.deepEqual(toLiveEvent({ event: "resync", data: "{}" }), { type: "resync" });
    assert.deepEqual(toLiveEvent({ event: "show", data: '{"show_id":"s","rev":9}' }), { type: "show", showId: "s", rev: 9 });
    assert.equal(toLiveEvent({ event: "show", data: "not json" }), null);
    assert.equal(toLiveEvent({ event: "mystery", data: "{}" }), null);
});

function harness({ opens, state = "authenticated" }) {
    const events = [];
    const sleeps = [];
    const session = { state, listeners: new Set(), subscribe(l) { session.listeners.add(l); return () => session.listeners.delete(l); } };
    let call = 0;
    let live;
    live = new LiveUpdates({
        open: async signal => { const result = opens[Math.min(call++, opens.length - 1)]; const value = await result(signal, live); return value; },
        session,
        onEvent: event => events.push(event),
        sleep: async ms => { sleeps.push(ms); await new Promise(resolve => setImmediate(resolve)); }
    });
    return { live, events, sleeps, session, calls: () => call };
}
const respond = chunks => async () => ({ body: streamOf(chunks) });
const until = async (condition) => { for (let i = 0; i < 200 && !condition(); i++) await new Promise(resolve => setImmediate(resolve)); };

test("events are delivered and a closed stream is reopened straight away", async () => {
    const { live, events, calls, sleeps } = harness({
        opens: [
            respond(["event: ready\ndata: {}\n\nevent: show\ndata: {\"show_id\":\"a\",\"rev\":1}\n\n"]),
            respond(["event: ready\ndata: {}\n\nevent: access\ndata: {}\n\n"])
        ]
    });
    live.start();
    await until(() => events.length >= 4);
    live.stop();
    assert.deepEqual(events.map(e => e.type), ["ready", "show", "ready", "access"]);
    assert.ok(calls() >= 2);
    assert.equal(sleeps[0], 250, "token-expiry closes are retried almost immediately");
});

test("failures back off exponentially up to a cap, and recover after a good connection", async () => {
    const fail = async () => { throw new NetworkError(); };
    const { live, sleeps, events } = harness({ opens: [fail, fail, fail, respond(["event: ready\ndata: {}\n\n"])] });
    live.start();
    await until(() => events.length >= 1);
    live.stop();
    assert.deepEqual(sleeps.slice(0, 3), [1000, 2000, 4000]);
});

test("a rejected login is not retried aggressively", async () => {
    const { live, sleeps, calls } = harness({ opens: [async () => { throw new ApiError(401, "expired"); }] });
    live.start();
    await until(() => sleeps.length >= 1);
    live.stop();
    assert.equal(sleeps[0], 30_000);
    assert.equal(calls(), 1);
});

test("nothing is opened while signed out", async () => {
    const { live, calls, sleeps } = harness({ opens: [respond([])], state: "unauthenticated" });
    live.start();
    await until(() => sleeps.length >= 1);
    live.stop();
    assert.equal(calls(), 0);
});

test("an announced revision the page already has is not news (the change bus carries the revision)", () => {
    const bus = new ChangeBus();
    const seen = [];
    bus.subscribe(change => seen.push(change));
    bus.emitExternal("a", 5);
    bus.emitExternal(null);
    assert.deepEqual(seen, [{ showId: "a", rev: 5 }, { showId: null }]);
});
