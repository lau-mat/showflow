import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function createContext(datasets = []) {
    let now = new Date(2026, 9, 25, 9, 5, 3);
    const callbacks = new Map();
    const cleared = [];
    let nextId = 1;
    let delay;
    const timers = datasets.map(dataset => ({ dataset, children: [] }));
    const context = vm.createContext({
        Date: class extends Date {
            constructor() { super(now.getTime()); }
        },
        document: { querySelectorAll: () => timers },
        dynamicGenerator(definition, timer) {
            const element = {
                className: definition.classes, textContent: definition.text,
                children: [], style: {}, attributes: { ...definition.attributes },
                setAttribute(name, value) { this.attributes[name] = value; }
            };
            timer.children.push(element);
            return element;
        },
        setInterval(callback, milliseconds) {
            const id = nextId++;
            callbacks.set(id, callback);
            delay = milliseconds;
            return id;
        },
        clearInterval(id) {
            cleared.push(id);
            callbacks.delete(id);
        }
    });
    vm.runInContext(readFileSync(new URL("../src/js/timers.js", import.meta.url), "utf8"), context);
    return {
        context, timers,
        tick: () => callbacks.forEach(callback => callback()),
        tickTimer: index => callbacks.get(index + 1)(),
        get intervalCount() { return callbacks.size; },
        setTime: date => { now = date; },
        get delay() { return delay; },
        get cleared() { return cleared; }
    };
}

test("formats selected, reordered, and repeated components with padding", () => {
    const { context } = createContext();
    const parts = { HH: 9, MM: 5, SS: 3 };
    assert.equal(context.formatTimerTime(parts), "09:05:03");
    assert.equal(context.formatTimerTime(parts, "MM-SS"), "05:03");
    assert.equal(context.formatTimerTime(parts, "SS:HH:MM"), "03:09:05");
    assert.equal(context.formatTimerTime(parts, "HH"), "09");
    assert.equal(context.formatTimerTime(parts, "HH:HH"), "09:09");
});

test("each timer updates independently and cleanup stops all updates", () => {
    const app = createContext([{ source: "system-clock" }, { source: "system-clock" }]);
    const stop = app.context.startTimers();
    app.setTime(new Date(2026, 9, 25, 9, 5, 4));
    app.tickTimer(0);
    assert.equal(app.timers[0].children[1].textContent, "09:05:04");
    assert.equal(app.timers[1].children[1].textContent, "09:05:03");
    app.tickTimer(1);
    assert.equal(app.timers[1].children[1].textContent, "09:05:04");
    stop();
    app.setTime(new Date(2026, 9, 25, 9, 5, 5));
    app.tick();
    assert.equal(app.timers[0].children[1].textContent, "09:05:04");
    assert.equal(app.intervalCount, 0);
});

test("countdown warning uses the exact 15-second threshold and clears on reset", () => {
    const app = createContext([{ source: "current-item", format: "MM-SS" }]);
    const start = new Date(2026, 9, 25, 9, 5, 3).getTime() / 1000;
    const state = {
        showData: { lines: [{ order: 1, duration: 60 }] },
        sessionData: { current_cue: 1, cue_started_at: start }
    };
    app.context.startTimers(app.context.document, () => state);
    const timer = app.timers[0];
    for (const [elapsed, warning, overdue] of [
        [44.9, "false", "false"],
        [45, "true", "false"],
        [45.1, "true", "false"],
        [59.9, "true", "false"],
        [60, "false", "false"],
        [61, "false", "true"]
    ]) {
        app.setTime(new Date((start + elapsed) * 1000));
        app.tick();
        assert.equal(timer.dataset.warning, warning, `warning at ${elapsed}`);
        assert.equal(timer.dataset.overdue, overdue, `overdue at ${elapsed}`);
        const percent = parseFloat(timer.children[2].children[0].style.width);
        assert.ok(Math.abs(percent - Math.max(0, (60 - elapsed) / 60 * 100)) < 0.001);
    }
    state.sessionData.cue_started_at = start + 61;
    app.tick();
    assert.equal(timer.dataset.warning, "false");
    assert.equal(timer.dataset.overdue, "false");
    assert.equal(timer.children[2].children[0].style.width, "100%");
    state.sessionData.current_cue = null;
    app.tick();
    assert.equal(timer.dataset.warning, "false");
});

test("renders immediately and updates all timers from the current wall clock", () => {
    const app = createContext([
        { source: "system-clock", label: "Current Time" },
        { source: "system-clock", format: "MM-SS" }
    ]);
    const stop = app.context.startTimers();
    assert.equal(app.timers[0].children[0].textContent, "Current Time");
    assert.equal(app.timers[0].children[1].textContent, "09:05:03");
    assert.equal(app.timers[1].children[1].textContent, "05:03");
    assert.equal(app.delay, 100);
    assert.equal(app.intervalCount, 2);

    app.setTime(new Date(2026, 9, 26, 0, 0, 1));
    app.tick();
    assert.equal(app.timers[0].children[1].textContent, "00:00:01");
    assert.equal(app.timers[1].children[1].textContent, "00:01");
    assert.equal(app.timers[0].children.length, 2);
    stop();
    assert.deepEqual(app.cleared, [1, 2]);
    assert.equal(app.intervalCount, 0);
});

test("rereads dynamic formats and labels without recreating the timer", () => {
    const app = createContext([{ source: "system-clock", format: "HH-MM-SS" }]);
    app.context.startTimers();
    app.timers[0].dataset.format = "SS:MM";
    app.timers[0].dataset.label = "Updated label";
    app.tick();
    assert.equal(app.timers[0].children[1].textContent, "03:05");
    assert.equal(app.timers[0].children[0].textContent, "Updated label");
    delete app.timers[0].dataset.format;
    app.tick();
    assert.equal(app.timers[0].children[1].textContent, "09:05:03");
});

test("additional time sources reuse rendering and formatting", () => {
    const app = createContext([{ source: "test-source", format: "MM-SS" }]);
    vm.runInContext('timerSources["test-source"] = () => ({ HH: 0, MM: 2, SS: 30 })', app.context);
    app.context.startTimers();
    assert.equal(app.timers[0].children[1].textContent, "02:30");
    app.timers[0].dataset.source = "system-clock";
    app.tick();
    assert.equal(app.timers[0].children[1].textContent, "05:03");
});

test("unknown sources fail explicitly and empty pages do not schedule updates", () => {
    for (const source of ["missing", "toString"]) {
        const app = createContext([{ source }]);
        assert.throws(() => app.context.startTimers(), /Unknown timer source/);
    }
    const app = createContext();
    app.context.startTimers()();
    assert.equal(app.delay, undefined);
});

test("actual show elapsed and item countdown follow live session state, not scheduled times", () => {
    const app = createContext([
        { source: "show-duration", format: "MM-SS" },
        { source: "current-item", format: "MM-SS" }
    ]);
    let state = {};
    app.context.startTimers(app.context.document, () => state);
    const values = () => app.timers.map(timer => timer.children[1].textContent);
    assert.deepEqual(values(), ["--:--", "--:--"]);

    const start = new Date(2026, 9, 25, 8, 0, 3).getTime() / 1000;
    state = {
        showData: {
            show: { time: start - 86400 },
            lines: [
                { order: 2, duration: 120, absolute_time: start + 3900 },
                { order: 1, duration: 60, absolute_time: null },
                { order: 3, duration: 30, absolute_time: null }
            ]
        },
        sessionData: { current_cue: 2, show_started_at: start, cue_started_at: start + 3900 }
    };
    app.tick();
    assert.deepEqual(values(), ["65:00", "02:00"]);
    app.setTime(new Date((start + 4021) * 1000));
    app.tick();
    assert.deepEqual(values(), ["67:01", "-00:01"]);

    state.sessionData.current_cue = 3;
    state.sessionData.cue_started_at = start + 4021;
    app.tick();
    assert.deepEqual(values(), ["67:01", "00:30"]);
    state.sessionData.current_cue = 1;
    state.sessionData.cue_started_at = start + 4021;
    app.tick();
    assert.deepEqual(values(), ["67:01", "01:00"]);

    state.sessionData.current_cue = null;
    state.sessionData.show_started_at = null;
    state.sessionData.cue_started_at = null;
    app.tick();
    assert.deepEqual(values(), ["00:00", "--:--"]);
});

test("restored session timestamps preserve elapsed time after a page reload", () => {
    const app = createContext([
        { source: "show-duration", format: "MM-SS" },
        { source: "current-item", format: "MM-SS" }
    ]);
    const now = new Date(2026, 9, 25, 9, 5, 3).getTime() / 1000;
    app.context.startTimers(app.context.document, () => ({
        showData: { lines: [{ order: 1, duration: 60 }] },
        sessionData: { current_cue: 1, show_started_at: now - 90, cue_started_at: now - 15 }
    }));
    assert.equal(app.timers[0].children[1].textContent, "01:30");
    assert.equal(app.timers[1].children[1].textContent, "00:45");
});

test("duration formatting supports hours, total minutes, zero, and invalid durations", () => {
    const { context } = createContext();
    const parts = context.timerDurationParts(3901);
    assert.equal(context.formatTimerTime(parts), "01:05:01");
    assert.equal(context.formatTimerTime(parts, "MM-SS"), "65:01");
    assert.equal(context.formatTimerTime(context.timerDurationParts(0), "MM-SS"), "00:00");
    assert.throws(() => context.timerDurationParts(NaN), /Invalid timer duration/);
});

test("item progress drains, turns overdue, and resets on cue changes", () => {
    const app = createContext([{ source: "current-item", label: "Current Item", format: "MM-SS" }]);
    const start = new Date(2026, 9, 25, 9, 5, 3).getTime() / 1000;
    const state = {
        showData: { lines: [{ order: 1, duration: 60 }, { order: 2, duration: 0 }] },
        sessionData: { current_cue: 1, cue_started_at: start }
    };
    app.context.startTimers(app.context.document, () => state);
    const timer = app.timers[0];
    const progress = timer.children[2];
    const fill = progress.children[0];
    assert.equal(fill.style.width, "100%");
    assert.equal(progress.attributes["aria-valuenow"], "100");
    assert.equal(progress.attributes["aria-label"], "Current Item time remaining");

    app.setTime(new Date((start + 30) * 1000));
    app.tick();
    assert.equal(fill.style.width, "50%");
    assert.equal(timer.dataset.overdue, "false");
    app.setTime(new Date((start + 60) * 1000));
    app.tick();
    assert.equal(fill.style.width, "0%");
    assert.equal(timer.dataset.overdue, "false");
    app.setTime(new Date((start + 61) * 1000));
    app.tick();
    assert.equal(timer.dataset.overdue, "true");
    assert.equal(fill.style.width, "0%");
    assert.equal(progress.attributes["aria-valuetext"], "Over time");

    state.sessionData.cue_started_at = start + 61;
    app.tick();
    assert.equal(fill.style.width, "100%");
    assert.equal(timer.dataset.overdue, "false");
    assert.equal(timer.children.length, 3);
    state.sessionData.current_cue = 2;
    app.tick();
    assert.equal(fill.style.width, "0%");
    assert.equal(progress.attributes["aria-valuenow"], "0");
    state.sessionData.current_cue = null;
    app.tick();
    assert.equal(progress.hidden, true);
    assert.equal(timer.dataset.overdue, "false");

    timer.dataset.source = "system-clock";
    app.tick();
    assert.equal(progress.hidden, true);
});
