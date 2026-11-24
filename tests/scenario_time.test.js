import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const scenarioTime = await import("../frontend/shared/js/scenario_time.ts");
const editShow = await import("../frontend/main/js/pages/edit_show.page.ts");

// client.ts opens its websocket and registers listeners on import.
globalThis.window = { location: { hostname: "localhost" } };
globalThis.WebSocket = class {};
globalThis.document = { addEventListener() {} };
const client = await import("../frontend/session/js/client.ts");

const showStart = new Date(2026, 9, 15, 20, 0, 0).getTime() / 1000;
const data = {
    show: { name: "Test show", time: showStart },
    roles: [{ id: 1, name: "Lighting" }],
    comments: [],
    lines: [
        { id: 1, name: "First", duration: 60, absolute_time: null },
        { id: 2, name: "Second", duration: 120, absolute_time: null },
        { id: 3, name: "Anchor", duration: 30, absolute_time: showStart + 600 },
        { id: 4, name: "Following", duration: 0, absolute_time: null },
        { id: 5, name: "Zero duration", duration: 45, absolute_time: null }
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
        editShow.generateAndPrintSheet(data, { printMode, roleSelect: "1" });
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
    globalThis.__invoke = async command => {
        calls.push(command);
        return command === "get_full_show_details" ? data : undefined;
    };
    await editShow.deleteLine(1);
    assert.deepEqual(calls, ["delete_scenario_line", "get_full_show_details"]);
});
