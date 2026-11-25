import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const t = await import("../frontend/shared/js/time_math.ts");

test("seconds split into and join from hours, minutes and seconds", () => {
    assert.deepEqual(t.splitSeconds(3723), { h: 1, m: 2, s: 3 });
    assert.deepEqual(t.splitSeconds(0), { h: 0, m: 0, s: 0 });
    assert.deepEqual(t.splitSeconds(-5), { h: 0, m: 0, s: 0 });
    assert.equal(t.joinSeconds(1, 2, 3), 3723);
    for (const total of [0, 59, 60, 3599, 3600, 86399, 604800]) {
        const { h, m, s } = t.splitSeconds(total);
        assert.equal(t.joinSeconds(h, m, s), total);
    }
});

test("clock times stay within a day and durations within the one-week limit the server enforces", () => {
    assert.equal(t.clampSeconds(-10, "clock"), 0);
    assert.equal(t.clampSeconds(90_000, "clock"), 86_399);
    assert.equal(t.clampSeconds(90_000, "duration"), 90_000);
    assert.equal(t.clampSeconds(10 ** 9, "duration"), 604_800);
    assert.equal(t.clampSeconds(Number.NaN, "duration"), 0);
    assert.equal(t.clampSeconds(12.6, "duration"), 13);
    assert.equal(t.MAX_DURATION_SECONDS, 7 * 24 * 3600);
});

test("segments wrap around like a dial", () => {
    assert.equal(t.stepSegment(59, 1, 59), 0);
    assert.equal(t.stepSegment(0, -1, 59), 59);
    assert.equal(t.stepSegment(23, 1, 23), 0);
    assert.equal(t.stepSegment(5, 10, 59), 15);
    assert.equal(t.stepSegment(55, 10, 59), 5);
    assert.equal(t.stepSegment(0, -61, 59), 59);
    assert.deepEqual([0, 1, 2].map(i => t.segmentMax("clock", i)), [23, 59, 59]);
    assert.deepEqual([0, 1, 2].map(i => t.segmentMax("duration", i)), [168, 59, 59]);
});

test("typed segment text is cleaned and capped", () => {
    assert.equal(t.parseSegment("07", 59), 7);
    assert.equal(t.parseSegment("", 59), 0);
    assert.equal(t.parseSegment("abc", 59), 0);
    assert.equal(t.parseSegment("99", 59), 59);
    assert.equal(t.parseSegment("9x9", 168), 99);
    assert.equal(t.parseSegment("1234", 168), 123 > 168 ? 168 : 123, "only as many digits as the maximum has");
    assert.equal(t.pad(5), "05");
    assert.equal(t.pad(168), "168");
    assert.equal(t.formatDuration(65), "1:05");
    assert.equal(t.formatDuration(3725), "1:02:05");
});

test("a month grid is always six Monday-first weeks that contain the month", () => {
    // December 2026 starts on a Tuesday and has 31 days.
    const grid = t.monthGrid(2026, 11);
    assert.equal(grid.length, 42);
    assert.deepEqual(grid.slice(0, 3).map(d => d.key), ["2026-11-30", "2026-12-01", "2026-12-02"]);
    assert.equal(grid[0].inMonth, false);
    assert.deepEqual(grid.filter(d => d.inMonth).map(d => d.day), Array.from({ length: 31 }, (_, i) => i + 1));
    assert.equal(new Date(grid[0].key + "T12:00:00").getDay(), 1, "starts on a Monday");
    // A month that starts on a Monday has no leading days; February in a leap year has 29.
    assert.equal(t.monthGrid(2027, 1)[0].key, "2027-02-01");
    assert.equal(t.monthGrid(2028, 1).filter(d => d.inMonth).length, 29);
    // January 2026 begins on a Thursday and the grid reaches back into December.
    assert.equal(t.monthGrid(2026, 0)[0].key, "2025-12-29");
    // Month arithmetic rolls over years (December + 1, January - 1).
    assert.equal(t.monthGrid(2026, 12)[10].key.startsWith("2027-01"), true);
});

test("date keys round trip and months are zero based", () => {
    assert.equal(t.dateKey(new Date(2026, 11, 5)), "2026-12-05");
    assert.deepEqual(t.parseDateKey("2026-12-05"), { year: 2026, month: 11, day: 5 });
});

test("a date and a time of day combine into local epoch seconds and back", () => {
    const epoch = t.composeDateTime("2026-12-01", 19 * 3600 + 30 * 60);
    assert.equal(new Date(epoch * 1000).getHours(), 19);
    assert.equal(new Date(epoch * 1000).getMinutes(), 30);
    assert.equal(new Date(epoch * 1000).getDate(), 1);
    assert.deepEqual(t.decomposeDateTime(epoch), { date: "2026-12-01", secondsOfDay: 19 * 3600 + 30 * 60 });
    // Changing only the date keeps the time of day, and the other way round.
    const moved = t.composeDateTime("2027-03-14", t.decomposeDateTime(epoch).secondsOfDay);
    assert.deepEqual(t.decomposeDateTime(moved), { date: "2027-03-14", secondsOfDay: 70_200 });
    assert.equal(t.decomposeDateTime(t.composeDateTime("2026-12-01", 99_999)).secondsOfDay, 86_399, "the time of day is clamped");
});

test("the suggested show time is the next full hour", () => {
    const at = new Date(2026, 11, 1, 18, 42, 10).getTime();
    const suggestion = new Date(t.nextFullHour(at) * 1000);
    assert.deepEqual([suggestion.getHours(), suggestion.getMinutes(), suggestion.getSeconds()], [19, 0, 0]);
    const late = new Date(t.nextFullHour(new Date(2026, 11, 1, 23, 59).getTime()) * 1000);
    assert.deepEqual([late.getDate(), late.getHours()], [2, 0], "rolls over midnight");
    const exact = new Date(t.nextFullHour(new Date(2026, 11, 1, 19, 0, 0).getTime()) * 1000);
    assert.equal(exact.getHours(), 20, "never the current moment");
});
