import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const { refreshCallerAccess, RENEW_WHEN_LEFT_SECONDS } = await import("../frontend/main/js/sync/caller_access.ts");
const { ApiError, NetworkError } = await import("../frontend/main/js/api/types.ts");

const NOW = 1_800_000_000;
const DAY = 24 * 60 * 60;

function setup({ passes = [], routes = {} } = {}) {
    const log = [];
    const local = {
        state: async () => ({ device_key: "device-key", passes }),
        savePass: async (...args) => { log.push(["save", ...args]); },
        saveTrust: async (...args) => { log.push(["trust", ...args]); }
    };
    const all = {
        "caller-passes/key": () => ({ public_key: "cloud-key" }),
        ...routes
    };
    const requests = [];
    const rest = {
        async request(path, options = {}) {
            requests.push([path, options.body]);
            if (all[path]) return all[path]();
            const show = path.match(/^shows\/([^/]+)\/caller-pass$/)?.[1];
            if (show) return { pass: `pass-${show}`, expires_at: NOW + 14 * DAY };
            throw new Error(`unexpected request ${path}`);
        }
    };
    const run = index => refreshCallerAccess(index, { rest, local, now: () => NOW * 1000 });
    return { run, log, requests };
}

const people = (members, callers) => ({ members, callers });

test("the host's trust data and a pass for every show of the account are stored", async () => {
    const { run, log, requests } = setup();
    await run([{ id: "a", ...people(["me", "sam"], ["me"]) }, { id: "b", ...people(["boss", "me"], ["boss"]) }]);
    assert.deepEqual(log, [
        ["trust", "cloud-key", { a: people(["me", "sam"], ["me"]), b: people(["boss", "me"], ["boss"]) }],
        // Calling or not: a pass also admits a member to a protected session.
        ["save", "a", "pass-a", NOW + 14 * DAY],
        ["save", "b", "pass-b", NOW + 14 * DAY]
    ]);
    assert.deepEqual(requests.at(-1), ["shows/b/caller-pass", { device_key: "device-key" }]);
});

test("passes are renewed only when they get old", async () => {
    const { run, log } = setup({
        passes: [
            { show_id: "fresh", expires_at: NOW + RENEW_WHEN_LEFT_SECONDS + DAY },
            { show_id: "old", expires_at: NOW + RENEW_WHEN_LEFT_SECONDS - DAY }
        ]
    });
    await run([{ id: "fresh" }, { id: "old" }]);
    assert.deepEqual(log.slice(1), [["save", "old", "pass-old", NOW + 14 * DAY]]);
});

test("a server without account access is left alone", async () => {
    const { run, log, requests } = setup({ routes: { "caller-passes/key": () => { throw new ApiError(404, "Not found"); } } });
    await run([{ id: "a", members: [], callers: [] }]);
    assert.deepEqual(log, []);
    assert.equal(requests.length, 1);
});

test("a right withdrawn mid-sync skips that show; other failures are reported", async () => {
    const withdrawn = setup({ routes: { "shows/a/caller-pass": () => { throw new ApiError(403, "no"); } } });
    await withdrawn.run([{ id: "a" }, { id: "b" }]);
    assert.deepEqual(withdrawn.log.filter(entry => entry[0] === "save").map(entry => entry[1]), ["b"]);

    const offline = setup({ routes: { "caller-passes/key": () => { throw new NetworkError(); } } });
    await assert.rejects(offline.run([]), NetworkError);
});
