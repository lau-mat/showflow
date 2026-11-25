import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const { RestClient, createRestDataApi } = await import("../frontend/main/js/api/rest.ts");
const { ApiError } = await import("../frontend/main/js/api/types.ts");
const { compareLines, sortCommentsByRole, canEditContent, canComment, canManageShow } = await import("../frontend/shared/js/show_types.ts");
const { newOp } = await import("../frontend/shared/js/sync_types.ts");

function setup(respond) {
    const requests = [];
    globalThis.fetch = async (url, init) => {
        requests.push({ url, method: init.method, body: init.body ? JSON.parse(init.body) : undefined });
        return respond(url, init, requests.length);
    };
    const client = new RestClient({
        baseUrl: "/api", clientHeaders: () => ({}), getAccessToken: async () => "tok", credentials: "same-origin"
    });
    return { api: createRestDataApi(client), requests };
}

const ok = body => ({ status: 200, ok: true, json: async () => body });
const applied = (op, rev = 1, extra = {}) => ({ status: "applied", op_id: op.op_id, rev, ...extra });

test("every edit on the web is an op pushed to the show's ops endpoint", async () => {
    const { api, requests } = setup(async (_url, init) => {
        const { ops } = JSON.parse(init.body);
        return ok({ results: ops.map(op => applied(op, 1, op.place ? { position: "k000001V" } : {})), rev: 1 });
    });

    const show = await api.newShow("Gala", 1_800_000_000);
    assert.equal(show.role, "owner");
    assert.match(show.id, /^[0-9a-f-]{36}$/);
    assert.equal(requests[0].url, `/api/shows/${show.id}/ops`);
    assert.deepEqual(requests[0].body.ops[0], {
        op_id: requests[0].body.ops[0].op_id, entity: "show", id: show.id, kind: "set", fields: { name: "Gala", time: 1_800_000_000 }
    });

    const role = await api.addRole(show.id, "Audio");
    assert.deepEqual(requests[1].body.ops[0].fields, { name: "Audio" });
    assert.equal(role.show_id, show.id);

    const after = crypto.randomUUID();
    const line = await api.addLine({ showId: show.id, after, lineName: "Intro", lineNote: "n", duration: 30, absoluteTime: null });
    const lineOp = requests[2].body.ops[0];
    assert.deepEqual(lineOp.place, { after });
    assert.deepEqual(lineOp.fields, { name: "Intro", note: "n", duration: 30, absolute_time: null });
    assert.equal(line.position, "k000001V", "the server's resolved position is returned");

    await api.addComment(show.id, line.id, null, "careful");
    assert.deepEqual(requests[3].body.ops[0].fields, { line_id: line.id, role_id: null, comment: "careful" });
    await api.editComment(show.id, "comment-id", role.id, "changed");
    assert.deepEqual(requests[4].body.ops[0].fields, { role_id: role.id, comment: "changed" });
    assert.equal(requests[4].body.ops[0].id, "comment-id");

    await api.deleteLine(show.id, line.id);
    assert.equal(requests[5].body.ops[0].kind, "delete");
    await api.deleteShow(show.id);
    assert.equal(requests[6].body.ops[0].entity, "show");
    assert.ok(requests.every(r => r.method === "POST"));
});

test("reads fetch the list and snapshots", async () => {
    const { api, requests } = setup(async () => ok([]));
    await api.getShows();
    await api.getShowDetails("abc");
    assert.deepEqual(requests.map(r => [r.method, r.url]), [["GET", "/api/shows"], ["GET", "/api/shows/abc"]]);
});

test("an op the server rejects surfaces as an error the page can show", async () => {
    const { api } = setup(async (_url, init) => {
        const [op] = JSON.parse(init.body).ops;
        return ok({ results: [{ status: "rejected", op_id: op.op_id, error: "You may not change this show" }], rev: 3 });
    });
    await assert.rejects(api.addRole("show", "Audio"), error => error instanceof ApiError && error.status === 422 && /may not change/.test(error.message));
});

test("script order is the plain string order of positions, with the id as tie-break", () => {
    const lines = [
        { id: "z", position: "b" }, { id: "y", position: "B" }, { id: "b", position: "a" },
        { id: "a", position: "a" }, { id: "c", position: "a0V" }
    ];
    assert.deepEqual(lines.sort(compareLines).map(l => l.id), ["y", "a", "b", "c", "z"]);
    // Not locale order: "a" < "B" there, which would swap the first and last keys' neighbours.
    assert.ok(compareLines({ id: "1", position: "B" }, { id: "1", position: "a" }) < 0);
});

test("general comments come first, then roles in the order they were listed", () => {
    const roles = [{ id: "r2" }, { id: "r1" }];
    const comments = [
        { id: "1", role_id: "r1" }, { id: "2", role_id: null }, { id: "3", role_id: "r2" }, { id: "4", role_id: null }
    ];
    assert.deepEqual(sortCommentsByRole(comments, roles).map(c => c.id), ["2", "4", "3", "1"]);
});

test("role permissions mirror the server", () => {
    assert.deepEqual(["owner", "editor", "commenter", "viewer"].map(canEditContent), [true, true, false, false]);
    assert.deepEqual(["owner", "editor", "commenter", "viewer"].map(canComment), [true, true, true, false]);
    assert.deepEqual(["owner", "editor", "commenter", "viewer"].map(canManageShow), [true, false, false, false]);
});

test("newOp makes a fresh idempotency key per op and only includes what it was given", () => {
    const a = newOp("show", "id", "delete");
    const b = newOp("show", "id", "delete");
    assert.notEqual(a.op_id, b.op_id);
    assert.deepEqual(Object.keys(a).sort(), ["entity", "id", "kind", "op_id"]);
    assert.deepEqual(newOp("line", "id", "set", { name: "x" }, { after: null }).place, { after: null });
});

test("editing a cue pushes only the fields that changed, under the server's field names", async () => {
    const { api, requests } = setup(async (_url, init) => {
        const { ops } = JSON.parse(init.body);
        return ok({ results: ops.map(op => applied(op)), rev: 1 });
    });
    await api.editLine("show-1", "line-1", { name: "New title", absoluteTime: 1_800_000_000 });
    await api.editLine("show-1", "line-1", { absoluteTime: null });
    const [first, second] = requests.map(r => r.body.ops[0]);
    assert.deepEqual([first.entity, first.id, first.kind], ["line", "line-1", "set"]);
    assert.deepEqual(first.fields, { name: "New title", absolute_time: 1_800_000_000 });
    assert.deepEqual(second.fields, { absolute_time: null }, "clearing a fixed time is an explicit null");
    assert.equal(requests[0].url, "/api/shows/show-1/ops");
});

test("editing the show pushes only the changed details as an op on the show itself", async () => {
    const { api, requests } = setup(async (_url, init) => {
        const { ops } = JSON.parse(init.body);
        return ok({ results: ops.map(op => applied(op)), rev: 1 });
    });
    await api.editShow("show-1", { name: "Renamed" });
    await api.editShow("show-1", { time: 1_800_000_000 });
    const [first, second] = requests.map(r => r.body.ops[0]);
    assert.deepEqual([first.entity, first.id, first.kind, first.fields], ["show", "show-1", "set", { name: "Renamed" }]);
    assert.deepEqual(second.fields, { time: 1_800_000_000 });
    assert.equal(requests[0].url, "/api/shows/show-1/ops");
});
