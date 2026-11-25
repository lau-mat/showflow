import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const scenarioTime = await import("../frontend/shared/js/scenario_time.ts");
const { buildProfile, installPlatform } = await import("../frontend/shared/js/platform.ts");
const { installApi } = await import("../frontend/main/js/api/index.ts");
const editShow = await import("../frontend/main/js/pages/edit_show.page.ts");
installPlatform(buildProfile("desktop", "linux"));

// client.ts opens its websocket and registers listeners on import.
globalThis.window = { location: { hostname: "localhost" } };
globalThis.WebSocket = class {};
globalThis.document = { addEventListener() {} };
const client = await import("../frontend/session/js/client.ts");

const showStart = new Date(2026, 9, 15, 20, 0, 0).getTime() / 1000;
const data = {
    show: { name: "Test show", time: showStart },
    roles: [{ id: "role-1", name: "Lighting" }],
    comments: [],
    lines: [
        { id: "line-1", position: "a", name: "First", duration: 60, absolute_time: null },
        { id: "line-2", position: "b", name: "Second", duration: 120, absolute_time: null },
        { id: "line-3", position: "c", name: "Anchor", duration: 30, absolute_time: showStart + 600 },
        { id: "line-4", position: "d", name: "Following", duration: 0, absolute_time: null },
        { id: "line-5", position: "e", name: "Zero duration", duration: 45, absolute_time: null }
    ]
};
const expected = ["20:00:00", "20:01:00", "20:10:00", "20:10:30", "20:10:30"];

test("relative starts accumulate previous durations and explicit starts reset the schedule", () => {
    const { resolveScenarioLineStartTimestamp, formatScenarioLineTime } = scenarioTime;
    data.lines.forEach((line, index) => {
        const start = resolveScenarioLineStartTimestamp(data.show.time, data.lines, index);
        assert.equal(formatScenarioLineTime(start), expected[index]);
    });
    assert.equal(data.lines[1].absolute_time, null);
});

test("times roll over midnight and accept numeric strings and epoch-zero anchors", () => {
    const context = scenarioTime;
    const start = new Date(2026, 9, 15, 23, 59, 30).getTime() / 1000;
    const lines = [{ duration: "60" }, { duration: 10 }];
    assert.equal(context.formatScenarioLineTime(
        context.resolveScenarioLineStartTimestamp(String(start), lines, 1)
    ), "00:00:30");
    assert.equal(context.resolveScenarioLineStartTimestamp(start, [{ absolute_time: 0 }], 0), 0);
    assert.throws(() => context.formatScenarioLineTime(null), /Unable to determine/);
});

test("editor and all print layouts display the same resolved start times", () => {
    const printContainer = { innerHTML: "", remove() {} };
    globalThis.window = { print() {} };
    globalThis.document = { getElementById: () => printContainer };

    const cells = editShow.buildEditShowLayout(data)
        .filter(item => item.classes === "line-number-cell");
    assert.deepEqual(cells.map(cell => cell.children[0].text), expected);

    for (const printMode of ["director", "single_role", "all_roles_separate"]) {
        editShow.generateAndPrintSheet(data, { printMode, roleSelect: "role-1" });
        const times = [...printContainer.innerHTML.matchAll(
            /class="col-time"><strong>(.*?)<\/strong>/g
        )].map(match => match[1]);
        assert.deepEqual(times, expected);
    }
});

test("client display and countdown share resolved start times", () => {
    client.session.showContent = data;
    data.lines.forEach((line, index) => {
        assert.equal(client.formatCueTime(data.lines, index), expected[index]);
        assert.equal(client.resolveCueStartTimestamp(data.lines, index),
            scenarioTime.resolveScenarioLineStartTimestamp(showStart, data.lines, index));
    });
});

test("deleting a line reloads show data to recalculate following starts", async () => {
    const calls = [];
    installApi({
        data: {
            subscribe: () => () => {},
            deleteLine: async (showId, id) => calls.push(["delete_line", showId, id]),
            getShowDetails: async id => { calls.push(["get_show_details", id]); return data; }
        }
    });
    await editShow.openEditShow("show-7");
    calls.length = 0;
    await editShow.deleteLine("line-1");
    assert.deepEqual(calls, [["delete_line", "show-7", "line-1"], ["get_show_details", "show-7"]]);
});

test("the print action is only offered on platforms that can print", () => {
    const hasPrint = () => editShow.buildEditShowLayout(data)
        .some(item => item.file === "icons/print.svg");
    assert.equal(hasPrint(), true);
    installPlatform(buildProfile("mobile", "android"));
    assert.equal(hasPrint(), false);
    installPlatform(buildProfile("desktop", "linux"));
});

test("the editor only offers what the member's role allows", () => {
    const icons = role => editShow.buildEditShowLayout({ ...data, show: { ...data.show, role } })
        .filter(item => item.type === "svg").map(item => item.file.replace("icons/", "").replace(".svg", ""));
    const count = (list, name) => list.filter(icon => icon === name).length;
    const lines = data.lines.length;

    const owner = icons("owner");
    assert.equal(count(owner, "add"), 2 + lines, "add role, add line, and an add-note per line");
    assert.equal(count(owner, "delete"), lines);
    assert.ok(owner.includes("share"));

    const editor = icons("editor");
    assert.equal(count(editor, "delete"), lines);
    assert.equal(count(editor, "add"), 2 + lines);

    const commenter = icons("commenter");
    assert.equal(count(commenter, "add"), lines, "only the add-note per line");
    assert.equal(count(commenter, "delete") + count(commenter, "edit"), 0);

    const viewer = icons("viewer");
    assert.equal(count(viewer, "add") + count(viewer, "delete") + count(viewer, "edit"), 0);
    assert.ok(viewer.includes("share"), "everyone can see who has access");
});
