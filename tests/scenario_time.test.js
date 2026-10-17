import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadScript(context, path) {
    vm.runInContext(readFileSync(new URL(`../src/${path}`, import.meta.url), "utf8"), context);
}

function createContext() {
    const context = vm.createContext({});
    loadScript(context, "js/scenario_time.js");
    return context;
}

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
    const context = createContext();
    data.lines.forEach((line, index) => {
        const start = context.resolveScenarioLineStartTimestamp(data.show.time, data.lines, index);
        assert.equal(context.formatScenarioLineTime(start), expected[index]);
    });
    assert.equal(data.lines[1].absolute_time, null);
});

test("times roll over midnight and accept numeric strings and epoch-zero anchors", () => {
    const context = createContext();
    const start = new Date(2026, 9, 15, 23, 59, 30).getTime() / 1000;
    const lines = [{ duration: "60" }, { duration: 10 }];
    assert.equal(context.formatScenarioLineTime(
        context.resolveScenarioLineStartTimestamp(String(start), lines, 1)
    ), "00:00:30");
    assert.equal(context.resolveScenarioLineStartTimestamp(start, [{ absolute_time: 0 }], 0), 0);
    assert.throws(() => context.formatScenarioLineTime(null), /Unable to determine/);
});

test("editor and all print layouts display the same resolved start times", () => {
    const context = createContext();
    const printContainer = { innerHTML: "", remove() {} };
    context.window = { __TAURI__: { core: { invoke() {} } }, print() {} };
    context.document = { getElementById: () => printContainer };
    loadScript(context, "js/pages/edit_show.page.js");

    const cells = context.buildEditShowLayout(data)
        .filter(item => item.classes === "line-number-cell");
    assert.deepEqual(Array.from(cells, cell => cell.text), expected);

    for (const printMode of ["director", "single_role", "all_roles_separate"]) {
        context.generateAndPrintSheet(data, { printMode, roleSelect: "1" });
        const times = [...printContainer.innerHTML.matchAll(
            /class="col-time"><strong>(.*?)<\/strong>/g
        )].map(match => match[1]);
        assert.deepEqual(times, expected);
    }
});

test("client display and countdown share resolved start times", () => {
    const context = createContext();
    context.window = { location: { hostname: "localhost" } };
    context.WebSocket = class {};
    context.document = { addEventListener() {} };
    loadScript(context, "js/client.js");
    context.showData = data;
    vm.runInContext("session.showContent = showData", context);
    data.lines.forEach((line, index) => {
        assert.equal(context.formatCueTime(data.lines, index), expected[index]);
        assert.equal(context.resolveCueStartTimestamp(data.lines, index),
            context.resolveScenarioLineStartTimestamp(showStart, data.lines, index));
    });
});

test("deleting a line reloads show data to recalculate following starts", async () => {
    const context = createContext();
    const calls = [];
    context.window = { __TAURI__: { core: { invoke: async command => calls.push(command) } } };
    loadScript(context, "js/pages/edit_show.page.js");
    context.refreshShowData = async () => calls.push("refresh");
    await context.deleteLine(1);
    assert.deepEqual(calls, ["delete_scenario_line", "refresh"]);
});
