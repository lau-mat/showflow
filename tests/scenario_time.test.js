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

    const editIcons = role => editShow.buildEditShowLayout({ ...data, show: { ...data.show, role } })
        .filter(item => item.file === "icons/edit.svg");
    assert.ok(editIcons("owner").every(item => typeof item.events?.click === "function"), "the edit icon does something");
    assert.equal(editIcons("owner").length, lines + 1, "one per cue, plus the one for the show's details");
    assert.equal(editIcons("editor").length, lines + 1);
    assert.equal(editIcons("commenter").length + editIcons("viewer").length, 0);
    const showEdit = editIcons("owner").find(item => item.attributes?.title === "Edit show details");
    assert.equal(typeof showEdit.events.click, "function");

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

test("the show details dialog starts from the show and saves only what changed", async () => {
    const edits = [];
    const emitted = [];
    let loads = 0;
    installApi({
        bus: { emitExternal: (...args) => emitted.push(args) },
        data: {
            subscribe: () => () => {},
            getShowDetails: async () => { loads++; return data; },
            editShow: async (showId, changes) => edits.push([showId, changes])
        }
    });
    await editShow.openEditShow("show-7");
    const click = editShow.buildEditShowLayout(data).find(item => item.attributes?.title === "Edit show details").events.click;

    let asked;
    globalThis.__dynamicPrompt = async options => { asked = options; return { showName: "Renamed show", showTime: String(data.show.time) }; };
    const before = loads;
    await click();
    assert.equal(asked.title, "Edit Show");
    const fields = Object.fromEntries(asked.elements.map(element => [element.id, element]));
    assert.deepEqual([fields.showName.value, fields.showTime.value, fields.showTime.type], ["Test show", data.show.time, "datetime-label"]);
    assert.deepEqual(edits, [["show-7", { name: "Renamed show" }]], "the unchanged time is not sent");
    assert.equal(loads, before + 1, "the editor reloads after saving");
    assert.deepEqual(emitted, [["show-7"]], "the show list behind it is told too");
    assert.deepEqual(asked.onValidate({ showName: "  ", showTime: "0" }), { showName: "Please fill in a name", showTime: "A start time must be selected" });
    assert.deepEqual(asked.onValidate({ showName: "ok", showTime: "5" }), {});

    // Nothing changed, or the dialog was cancelled: nothing is sent.
    globalThis.__dynamicPrompt = async () => ({ showName: "Test show", showTime: String(data.show.time) });
    await click();
    globalThis.__dynamicPrompt = async () => null;
    await click();
    assert.equal(edits.length, 1);

    // A new time is sent as epoch seconds, trimmed name included.
    globalThis.__dynamicPrompt = async () => ({ showName: "  Test show  ", showTime: String(data.show.time + 3600) });
    await click();
    assert.deepEqual(edits.at(-1), ["show-7", { time: data.show.time + 3600 }]);
});
