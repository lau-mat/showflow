const timerSources = {
    "system-clock": now => ({
        HH: now.getHours(),
        MM: now.getMinutes(),
        SS: now.getSeconds()
    }),
    "show-duration": (now, { sessionData }) => {
        if (!sessionData) return null;
        if (sessionData.show_started_at == null) return timerDurationParts(0);
        return timerDurationParts(Math.max(0, Math.floor(now.getTime() / 1000 - sessionData.show_started_at)));
    },
    "current-item": (now, { showData, sessionData }) => {
        if (!showData || !sessionData?.current_cue || sessionData.cue_started_at == null) return null;
        const lines = [...showData.lines].sort((a, b) => a.order - b.order);
        const index = sessionData.current_cue - 1;
        const line = lines[index];
        if (!line) return null;
        const duration = Number(line.duration);
        const remaining = sessionData.cue_started_at + duration - now.getTime() / 1000;
        return {
            ...timerDurationParts(Math.ceil(remaining)),
            overdue: remaining < 0,
            warning: remaining > 0 && remaining <= 15,
            remainingFraction: duration > 0 ? Math.max(0, Math.min(1, remaining / duration)) : 0
        };
    }
};

function timerDurationParts(seconds) {
    if (!Number.isFinite(seconds)) throw new Error("Invalid timer duration.");
    const absolute = Math.abs(seconds);
    return {
        HH: Math.floor(absolute / 3600),
        MM: Math.floor(absolute / 60) % 60,
        SS: absolute % 60,
        totalMinutes: Math.floor(absolute / 60),
        negative: seconds < 0
    };
}

function formatTimerTime(parts, format = "HH-MM-SS") {
    const formatted = format.replace(/-/g, ":").replace(/HH|MM|SS/g, token => {
        if (parts == null) return "--";
        const value = token === "MM" && !format.includes("HH") && parts.totalMinutes != null
            ? parts.totalMinutes : parts[token];
        return String(value).padStart(2, "0");
    });
    return parts?.negative ? `-${formatted}` : formatted;
}

function startTimers(root = document, getState = () => ({})) {
    const timers = Array.from(root.querySelectorAll(".timer"), timer => {
        const label = dynamicGenerator({
            type: "div", classes: "timerLabel", text: timer.dataset.label
        }, timer);
        const value = dynamicGenerator({
            type: "div", classes: "timerValue"
        }, timer);
        return { timer, label, value };
    });

    function updateTimer(entry) {
        const now = new Date();
        const state = getState();
        const { timer, label, value } = entry;
        const source = timerSources[timer.dataset.source];
        if (!Object.hasOwn(timerSources, timer.dataset.source)) {
            throw new Error(`Unknown timer source: ${timer.dataset.source}`);
        }

        const labelText = timer.dataset.label || "";
        if (label.textContent !== labelText) label.textContent = labelText;
        const parts = source(now, state);
        const text = formatTimerTime(parts, timer.dataset.format);
        if (value.textContent !== text) value.textContent = text;
        timer.dataset.overdue = String(parts?.overdue === true);
        timer.dataset.warning = String(parts?.warning === true);

        if (parts?.remainingFraction != null && !entry.progress) {
            entry.progress = dynamicGenerator({
                type: "div",
                classes: "timerProgress",
                attributes: { role: "progressbar", "aria-valuemin": "0", "aria-valuemax": "100" }
            }, timer);
            entry.fill = dynamicGenerator({
                type: "div", classes: "timerProgressFill",
                attributes: { "aria-hidden": "true" }
            }, entry.progress);
        }
        if (entry.progress) {
            entry.progress.hidden = parts?.remainingFraction == null;
            const percent = (parts?.remainingFraction ?? 0) * 100;
            entry.fill.style.width = `${percent}%`;
            entry.progress.setAttribute("aria-label", `${timer.dataset.label || "Timer"} time remaining`);
            entry.progress.setAttribute("aria-valuenow", String(Math.round(percent)));
            entry.progress.setAttribute("aria-valuetext",
                parts?.overdue ? "Over time" : `${Math.round(percent)}% remaining`);
        }
    }

    timers.forEach(updateTimer);
    const intervals = timers.map(entry => setInterval(() => updateTimer(entry), 100));
    return () => intervals.forEach(interval => clearInterval(interval));
}
