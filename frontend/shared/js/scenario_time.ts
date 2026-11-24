// Values may arrive as numeric strings (e.g. form data or session payloads).
type NumericInput = number | string | null | undefined;

export interface ScenarioTimingLine {
    absolute_time?: NumericInput;
    duration?: NumericInput;
}

export function resolveScenarioLineStartTimestamp(
    showStart: NumericInput,
    lines: readonly ScenarioTimingLine[],
    lineIndex: number
): number | null {
    let nextStart = Number(showStart);
    if (!Number.isFinite(nextStart)) return null;

    for (let index = 0; index <= lineIndex; index++) {
        const line = lines[index];
        const start = line.absolute_time == null ? nextStart : Number(line.absolute_time);
        if (!Number.isFinite(start)) return null;
        if (index === lineIndex) return start;
        nextStart = start + Number(line.duration || 0);
    }

    return null;
}

export function formatScenarioLineTime(timestamp: number | null | undefined): string {
    if (timestamp == null) throw new Error("Unable to determine scenario line start time.");
    return new Date(timestamp * 1000).toLocaleTimeString("en-GB", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false
    });
}

export function formatScenarioLineDuration(duration: number): string {
    const hours = Math.floor(duration / 3600);
    const minutes = Math.floor((duration % 3600) / 60);
    const seconds = duration % 60;

    // Pad minutes and seconds to always be 2 digits
    const paddedMinutes = String(minutes).padStart(2, '0');
    const paddedSeconds = String(seconds).padStart(2, '0');

    // Include hours only if present
    if (hours > 0) {
        // Minutes must be padded if hours exist (e.g., 1:02:20)
        return `${hours}:${paddedMinutes}:${paddedSeconds}`;
    }

    // If under an hour, don't pad single-digit minutes (e.g., 1:20 or 0:05)
    return `${minutes}:${paddedSeconds}`;
}
