import { dynamicGenerator, dynamicPrompt, generateDynamic, type ElementDefinition } from "/shared/js/dynamic_generator";
import { formatScenarioLineTime, resolveScenarioLineStartTimestamp } from "/shared/js/scenario_time";
import { SessionSocket } from "/shared/js/session_socket";
import type { SessionClient, SessionState } from "/shared/js/session_types";
import type { ScenarioLine, ShowDetails } from "/shared/js/show_types";

const ws = new SessionSocket();

let loginSend = false;

interface ClientSession {
    roleId: number | null;
    userName: string | null;
    clientId: string | null;
    name: string | null;
    showContent: ShowDetails | null;
    sessionData: SessionState | null;
    personalNotes: Record<string, string>;
}

export const session: ClientSession = {
    roleId: null,
    userName: null,
    clientId: null,
    name: null,
    showContent: null,
    sessionData: null,
    personalNotes: {}
};

function showContent(): ShowDetails {
    if (!session.showContent) throw new Error("Show data has not been received yet.");
    return session.showContent;
}

function refreshCueViews(): void {
    const lines = sortedLines();
    renderCueList(lines);
    renderCurrentCue(lines);
    renderNextCue(lines);
    renderCueStrip(lines);
}

ws.on("get_show_data", data => {
    session.showContent = data.data;

    if (session.roleId != null) return;
    if (session.sessionData) selectRole();
});

ws.on("get_session_data", data => {
    session.sessionData = data.session;

    if (session.roleId != null) {
        refreshCueViews();
        return;
    }
    if (session.showContent) selectRole();
});

ws.on("join_session", ({ command: _command, ...user }) => {
    session.sessionData?.joined_users.push(user);

    if (!loginSend) return;
    if (user.client_name !== session.userName || user.client_role !== session.roleId) return;
    loginSend = false;
    window.localStorage.setItem("client_id", user.client_id);
    session.clientId = user.client_id;
});

ws.on("cue_change", ({ current_cue }) => {
    if (!session.sessionData) return;
    session.sessionData.current_cue = current_cue;
    refreshCueViews();
});

ws.on("error", data => {
    throw new Error(`Backend error: ${data.message}`);
});

function checkAndRestoreExistingUser(): SessionClient | null {
    const previousClientId = window.localStorage.getItem("client_id");
    if (!previousClientId) return null;
    return session.sessionData?.joined_users.find(user => user.client_id === previousClientId) ?? null;
}

async function selectRole(): Promise<void> {
    const previousUser = checkAndRestoreExistingUser();
    if (previousUser !== null) {
        session.userName = previousUser.client_name;
        session.roleId = previousUser.client_role;
        session.clientId = previousUser.client_id;
    } else {
        const roleOptions: ElementDefinition[] = showContent().roles.map(role => (
            { type: "option", text: role.name, attributes: { value: role.id } }
        ));

        const joined = await dynamicPrompt({
            title: "Select role",
            confirmText: "Join",
            required: true,
            elements: [
                { type: "input-label", label: "Name", id: "userName" },
                { type: "select-label", label: "Role", id: "roleId", options: roleOptions }
            ]
        });
        if (!joined) return;
        const { userName, roleId } = joined;

        loginSend = true;
        session.roleId = Number(roleId);
        session.userName = String(userName);

        ws.emit("join_session", { user_name: String(userName), role_id: Number(roleId) });
    }

    buildStructure();
}

// ── Intervals ────────────────────────────────────────────────────────────────
let clockInterval: ReturnType<typeof setInterval> | null = null;
let countdownInterval: ReturnType<typeof setInterval> | null = null;

// ── Helpers ───────────────────────────────────────────────────────────────────
function sortedLines(): ScenarioLine[] {
    return [...showContent().lines].sort((a, b) => a.order - b.order);
}

function currentIndex(lines: ScenarioLine[]): number {
    const c = session.sessionData?.current_cue;
    if (c == null) return -1;
    return Math.min(c - 1, lines.length - 1);
}

function getComment(lineId: number, roleId: number | null): string | null {
    return (session.showContent?.comments ?? [])
        .find(c => c.line_id === lineId && c.role_id === roleId)?.comment ?? null;
}

function fmtCountdown(secs: number): string {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    return h > 0
        ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
        : `${m}:${String(s).padStart(2, "0")}`;
}

export function formatCueTime(lines: ScenarioLine[], cueIndex: number): string {
    return formatScenarioLineTime(resolveCueStartTimestamp(lines, cueIndex));
}

export function resolveCueStartTimestamp(lines: ScenarioLine[], cueIndex: number): number | null {
    return resolveScenarioLineStartTimestamp(session.showContent?.show.time, lines, cueIndex);
}

function noteNodes(lineId: number, small = false): ElementDefinition[] {
    const general = getComment(lineId, null);
    const personal = getComment(lineId, session.roleId);
    const nodes: ElementDefinition[] = [];
    const smClass = small ? ["ch-note-sm"] : [];
    if (general) nodes.push({
        type: "div", classes: ["ch-note", "ch-note-general", ...smClass],
        children: [
            { type: "span", classes: "ch-note-label", text: "GENERAL" },
            { type: "p", classes: "ch-note-body", html: general.replace(/\n/g, "<br>") }
        ]
    });
    if (personal) nodes.push({
        type: "div", classes: ["ch-note", "ch-note-personal", ...smClass],
        children: [
            { type: "span", classes: "ch-note-label", text: "MY NOTE" },
            { type: "p", classes: "ch-note-body", html: personal.replace(/\n/g, "<br>") }
        ]
    });
    return nodes;
}

function stateClasses(prefix: string, i: number, cur: number): string[] {
    return [
        prefix,
        ...(i < cur ? ["ch-cue-done"] : []),
        ...(i === cur ? ["ch-cue-active"] : []),
        ...(i === cur + 1 ? ["ch-cue-next"] : [])
    ];
}

// ── Sub-renderers ─────────────────────────────────────────────────────────────
function renderCueList(lines: ScenarioLine[]): void {
    const el = document.getElementById("chCueList");
    if (!el) return;
    el.replaceChildren();
    const cur = currentIndex(lines);
    lines.forEach((line, i) => {
        dynamicGenerator({
            type: "div",
            classes: stateClasses("ch-cue-item", i, cur),
            children: [
                { type: "span", classes: "ch-cue-num", text: String(i + 1).padStart(2, "0") },
                { type: "span", classes: "ch-cue-name", text: line.name },
                { type: "span", classes: "ch-cue-time", text: formatCueTime(lines, i) }
            ]
        }, el);
    });
}

function renderCurrentCue(lines: ScenarioLine[]): void {
    // Update header cue counter (visible on mobile)
    const counter = document.getElementById("chCueCounter");
    const idx = currentIndex(lines);
    if (counter) {
        counter.textContent = idx >= 0 ? `${idx + 1} / ${lines.length}` : `— / ${lines.length}`;
    }

    const el = document.getElementById("chCurrentWrap");
    if (!el) return;
    el.replaceChildren();
    if (idx < 0) {
        dynamicGenerator({ type: "div", classes: "ch-standby", text: "Waiting for show to start…" }, el);
        return;
    }
    const line = lines[idx];
    dynamicGenerator({
        type: "div", classes: "ch-current-card",
        children: [
            { type: "div", classes: "ch-cur-header", children: [
                { type: "span", classes: "ch-cur-badge", text: `CUE ${idx + 1}` },
                { type: "h2", classes: "ch-cur-title", text: line.name },
                { type: "span", classes: "ch-cur-time", text: formatCueTime(lines, idx) }
            ] },
            ...(line.comment ? [{ type: "p", classes: "ch-cur-desc", text: line.comment }] : []),
            ...noteNodes(line.id)
        ]
    }, el);

    // Trigger brief flash to signal cue change
    const card = el.querySelector<HTMLElement>(".ch-current-card");
    if (card) {
        card.classList.remove("ch-flash");
        void card.offsetWidth; // force reflow
        card.classList.add("ch-flash");
    }
}

function renderCueStrip(lines: ScenarioLine[]): void {
    const el = document.getElementById("chCueStrip");
    if (!el) return;
    el.replaceChildren();
    const cur = currentIndex(lines);
    lines.forEach((line, i) => {
        dynamicGenerator({
            type: "div",
            classes: stateClasses("ch-strip-item", i, cur),
            children: [
                { type: "span", classes: "ch-strip-num", text: String(i + 1).padStart(2, "0") },
                { type: "span", classes: "ch-strip-name", text: line.name }
            ]
        }, el);
    });
    // Auto-scroll active item into view
    el.querySelector(".ch-cue-active")
        ?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
}

function renderNextCue(lines: ScenarioLine[]): void {
    if (countdownInterval) { clearInterval(countdownInterval); countdownInterval = null; }
    const el = document.getElementById("chNextWrap");
    if (!el) return;
    el.replaceChildren();
    const nextIdx = currentIndex(lines) + 1;
    if (nextIdx >= lines.length) return;
    const line = lines[nextIdx];
    const cueStartTimestamp = resolveCueStartTimestamp(lines, nextIdx);

    dynamicGenerator({
        type: "div", classes: "ch-next-card",
        children: [
            { type: "div", classes: "ch-next-header", children: [
                { type: "span", classes: "ch-next-badge", text: "NEXT" },
                { type: "span", classes: "ch-next-name", text: line.name },
                { type: "div", classes: "ch-next-times", children: [
                    { type: "span", classes: "ch-next-time-label", text: formatCueTime(lines, nextIdx) },
                    ...(cueStartTimestamp != null ? [{ type: "span", classes: "ch-countdown", id: "chCountdown", text: "" }] : [])
                ] }
            ] },
            ...noteNodes(line.id, true)
        ]
    }, el);

    if (cueStartTimestamp != null) {
        const cdEl = document.getElementById("chCountdown");
        const tick = () => {
            if (cdEl) cdEl.textContent = fmtCountdown(Math.max(0, Math.floor(cueStartTimestamp - Date.now() / 1000)));
        };
        tick();
        countdownInterval = setInterval(tick, 1000);
    }
}

// ── Layout builder ────────────────────────────────────────────────────────────
function buildStructure(): void {
    if (clockInterval) { clearInterval(clockInterval); clockInterval = null; }
    if (countdownInterval) { clearInterval(countdownInterval); countdownInterval = null; }

    const existing = document.getElementById("chApp");
    const app = existing ?? dynamicGenerator({ type: "div", id: "chApp", classes: "ch-layout" }, document.body);
    app.replaceChildren();

    generateDynamic([
        { type: "header", classes: "ch-header", varId: "hdr" },
        { type: "div", classes: "ch-show-meta", target: "@hdr", varId: "meta" },
        { type: "span", classes: "ch-show-name", text: showContent().show.name, target: "@meta" },
        { type: "span", classes: "ch-live-badge", text: "LIVE", target: "@meta" },
        { type: "span", classes: "ch-cue-counter", id: "chCueCounter", text: "— / —", target: "@meta" },
        { type: "span", classes: "ch-clock", id: "chClock", target: "@hdr" },

        { type: "main", classes: "ch-body", varId: "body" },
        { type: "aside", classes: "ch-sidebar", target: "@body", varId: "sidebar" },
        { type: "div", classes: "ch-sidebar-head", text: "CUE LIST", target: "@sidebar" },
        { type: "div", classes: "ch-cue-list", id: "chCueList", target: "@sidebar" },

        { type: "div", classes: "ch-content", target: "@body", varId: "content" },
        { type: "div", id: "chCurrentWrap", target: "@content" },
        { type: "div", id: "chNextWrap", target: "@content" },

        { type: "div", classes: "ch-cue-strip", id: "chCueStrip" },
    ], app);

    refreshCueViews();

    const clockEl = document.getElementById("chClock");
    const tickClock = () => { if (clockEl) clockEl.textContent = new Date().toLocaleTimeString("en-GB"); };
    tickClock();
    clockInterval = setInterval(tickClock, 1000);
}

// ── Fullscreen on double-click / double-tap ───────────────────────────────
function toggleFullscreen(): void {
    if (!document.fullscreenElement) {
        document.documentElement.requestFullscreen().catch(() => {});
    } else {
        document.exitFullscreen().catch(() => {});
    }
}

// Desktop
document.addEventListener("dblclick", toggleFullscreen);

// Mobile — browsers don't reliably fire dblclick on touch
let lastTap = 0;
document.addEventListener("touchend", e => {
    const now = Date.now();
    if (now - lastTap < 300) {
        e.preventDefault();
        toggleFullscreen();
    }
    lastTap = now;
}, { passive: false });
