function resolveScenarioLineStartTimestamp(showStart, lines, lineIndex) {
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

function formatScenarioLineTime(timestamp) {
    if (timestamp == null) throw new Error("Unable to determine scenario line start time.");
    return new Date(timestamp * 1000).toLocaleTimeString("en-GB", {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false
    });
}
