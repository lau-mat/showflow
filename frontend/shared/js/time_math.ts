// Pure logic behind the time and date compound components (no DOM), so it can be tested.

export type TimeMode = "clock" | "duration";

/** Longest duration the editor accepts: one week (protocol::sync::MAX_DURATION_SECONDS). */
export const MAX_DURATION_SECONDS = 7 * 24 * 3600;
export const SECONDS_PER_DAY = 86_400;

export const pad = (n: number, width = 2): string => String(Math.trunc(n)).padStart(width, "0");

export function maxSeconds(mode: TimeMode): number {
    return mode === "clock" ? SECONDS_PER_DAY - 1 : MAX_DURATION_SECONDS;
}

export function clampSeconds(total: number, mode: TimeMode): number {
    if (!Number.isFinite(total)) return 0;
    return Math.min(Math.max(Math.round(total), 0), maxSeconds(mode));
}

export function splitSeconds(total: number): { h: number; m: number; s: number } {
    const whole = Math.max(0, Math.trunc(total));
    return { h: Math.floor(whole / 3600), m: Math.floor((whole % 3600) / 60), s: whole % 60 };
}

export function joinSeconds(h: number, m: number, s: number): number {
    return h * 3600 + m * 60 + s;
}

/** Largest value of the hours / minutes / seconds segment. */
export function segmentMax(mode: TimeMode, index: number): number {
    if (index > 0) return 59;
    return mode === "clock" ? 23 : MAX_DURATION_SECONDS / 3600;
}

/** Steps a segment up or down; it wraps around inside its own range, like a dial. */
export function stepSegment(value: number, delta: number, max: number): number {
    const size = max + 1;
    return (((value + delta) % size) + size) % size;
}

/** Reads what a person typed in a segment: digits only, never above the segment's maximum. */
export function parseSegment(text: string, max: number): number {
    const digits = text.replace(/\D/g, "").slice(0, String(max).length);
    return digits === "" ? 0 : Math.min(Number(digits), max);
}

export function formatDuration(total: number): string {
    const { h, m, s } = splitSeconds(total);
    return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

// ---- dates (all local time, like the rest of the app)

/** `YYYY-MM-DD` of a local date. */
export function dateKey(date: Date): string {
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function parseDateKey(key: string): { year: number; month: number; day: number } {
    const [year, month, day] = key.split("-").map(Number);
    return { year, month: month - 1, day };
}

export interface CalendarDay {
    key: string;
    day: number;
    inMonth: boolean;
}

/** Six full weeks (42 days) for the month, starting on Monday, so the grid never changes height. */
export function monthGrid(year: number, month: number): CalendarDay[] {
    const first = new Date(year, month, 1);
    const lead = (first.getDay() + 6) % 7; // Monday = 0
    return Array.from({ length: 42 }, (_, index) => {
        const date = new Date(year, month, 1 - lead + index);
        return { key: dateKey(date), day: date.getDate(), inMonth: date.getMonth() === month };
    });
}

/** Local epoch seconds for a date and a time of day. */
export function composeDateTime(key: string, secondsOfDay: number): number {
    const { year, month, day } = parseDateKey(key);
    const { h, m, s } = splitSeconds(clampSeconds(secondsOfDay, "clock"));
    return Math.floor(new Date(year, month, day, h, m, s).getTime() / 1000);
}

export function decomposeDateTime(epochSeconds: number): { date: string; secondsOfDay: number } {
    const date = new Date(epochSeconds * 1000);
    return { date: dateKey(date), secondsOfDay: joinSeconds(date.getHours(), date.getMinutes(), date.getSeconds()) };
}

/** The start of the next full hour: a sensible suggestion for "when is the show". */
export function nextFullHour(nowMs: number = Date.now()): number {
    const date = new Date(nowMs);
    date.setMinutes(0, 0, 0);
    date.setHours(date.getHours() + 1);
    return Math.floor(date.getTime() / 1000);
}

export function formatDateTime(epochSeconds: number, locale?: string): string {
    return new Date(epochSeconds * 1000).toLocaleString(locale, {
        weekday: "short", day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", hour12: false
    });
}
