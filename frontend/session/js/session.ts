import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { addHoverBox, dynamicGenerator, generateDynamic } from "/shared/js/dynamic_generator";
import { detectPlatform } from "/shared/js/platform";
import type { CallerInfo, SessionState } from "/shared/js/session_types";
import { compareLines, sortCommentsByRole, type ScenarioLine, type ShowDetails } from "/shared/js/show_types";
import { AccessFlow, openSessionSocket, pageNotice, showNotice } from "./claim";
import { startTimers } from "./timers";

const ws = await openSessionSocket();
// Set in the host computer's own caller window.
const hostContext = window.__SHOWFLOW_SESSION__;

let showData: ShowDetails | null = null;
let sessionData: SessionState | null = null;

function requireElement(id: string): HTMLElement {
    const element = document.getElementById(id);
    if (!element) throw new Error(`Missing element #${id}`);
    return element;
}

const elemShowName = requireElement("ShowName");
const elemItemCounter = requireElement("itemCounter");
const btnNextCue = requireElement("btnNextCue") as HTMLButtonElement;
const btnPreviousCue = requireElement("previousCue") as HTMLButtonElement;
const btnRelease = requireElement("releaseCaller") as HTMLButtonElement;

/** The buttons only work once the server has made this device a caller. */
function setControlsEnabled(enabled: boolean): void {
    btnNextCue.disabled = !enabled;
    btnPreviousCue.disabled = !enabled;
}
setControlsEnabled(false);
// The host's own window cannot give up the role: it is how the host calls.
btnRelease.hidden = Boolean(hostContext?.isHost);

export const callerFlow = new AccessFlow({
    kind: "call",
    send: claim => ws.emit("claim_caller", { claim }),
    onGranted: () => setControlsEnabled(true),
    onLost: reason => {
        setControlsEnabled(false);
        // Back to the join screen, which offers "Become show caller" again.
        pageNotice.set(reason ?? "You are no longer a show caller.");
        window.location.replace(`client.html${window.location.search}`);
    },
    notify: text => showNotice(text)
});

ws.on("welcome", message => {
    callerFlow.welcome(message);
    if (hostContext?.isHost) {
        void invoke<string>("caller_host_token", { sessionId: hostContext.sessionId })
            .then(token => callerFlow.claim({ method: "host", token }));
        return;
    }
    // Anyone else gets here by becoming caller on the join screen, which stored a token.
    if (!callerFlow.resume()) {
        pageNotice.set("Choose \"Become show caller\" to open the caller view.");
        window.location.replace(`client.html${window.location.search}`);
    }
});
ws.on("policy", rules => callerFlow.adopt(rules));
ws.on("caller_status", status => callerFlow.handle(status));
ws.on("callers", ({ list }) => renderCallers(list));

function renderCallers(list: CallerInfo[]): void {
    requireElement("callerList").textContent = list.length ? list.map(caller => caller.name).join(", ") : "—";
}

// Sharing (QR) and window fullscreen only exist where the platform provides them. (Nothing here
// may await before every message handler is registered: the host sends the show right away.)
async function addActionButtons(): Promise<void> {
    const profile = await detectPlatform();
    if (profile.features.qrShare && hostContext?.isHost) {
        dynamicGenerator({ type: "svg", file: "icons/share.svg", events: { click: share }, classes: ["btn", "icon-btn"], style: { width: "24px", height: "24px" } }, "#actionBtnContainer");
    }
    if (profile.features.fullscreenWindow) {
        dynamicGenerator({ type: "svg", file: "icons/fullScreen.svg", events: { click: toggleFullscreen }, classes: ["btn", "icon-btn"], style: { width: "24px", height: "24px", marginLeft: "5px" } }, "#actionBtnContainer");
    }
}

// Module scripts are deferred, so the DOM may already be parsed by now.
if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => { void addActionButtons(); });
} else {
    void addActionButtons();
}

ws.on("error", data => {
    throw new Error(`Backend error: ${data.message}`);
});

ws.on("cue_change", ({ current_cue, next_cue, show_started_at, cue_started_at, by }) => {
    if (!showData) return;
    // With several callers it matters who moved the show.
    if (by) requireElement("cueChangedBy").textContent = `by ${by}`;
    if (sessionData) {
        sessionData.current_cue = current_cue;
        sessionData.show_started_at = show_started_at;
        sessionData.cue_started_at = cue_started_at;
    }
    renderCueNotes();

    // Reset all cards
    document.querySelectorAll(".cue-card").forEach(el => {
        el.classList.remove("active", "next");
        const badge = el.querySelector(".cue-status");
        if (badge) badge.textContent = "pending";
    });

    const sorted = [...showData.lines].sort(compareLines);
    const activeId = sorted[current_cue - 1]?.id;
    const nextId = next_cue == null ? undefined : sorted[next_cue - 1]?.id;

    const activeCard = activeId != null ? document.querySelector(`.cue-card[data-line-id="${activeId}"]`) : null;
    if (activeCard) {
        activeCard.classList.add("active");
        const badge = activeCard.querySelector(".cue-status");
        if (badge) badge.textContent = "LIVE";
        activeCard.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }

    const nextCard = nextId != null ? document.querySelector(`.cue-card[data-line-id="${nextId}"]`) : null;
    if (nextCard) nextCard.classList.add("next");
});

async function toggleFullscreen(): Promise<void> {
    // Resolved lazily: only available inside Tauri, not in a plain browser tab.
    const appWindow = getCurrentWindow();
    const isFullscreen = await appWindow.isFullscreen();
    await appWindow.setFullscreen(!isFullscreen);
}

async function share(): Promise<void> {
    try {
        const qrSvg = await invoke<string>("session_qr", { sessionId: hostContext?.sessionId });

        addHoverBox(
            null,
            "Share session",
            [
                { type: "div", id: "qr-container", classes: "qr-box" },
                {
                    type: "svg",
                    sketch: qrSvg,
                    pathStyle: {
                        path: { fill: "rgb(140, 140, 140)" },
                        rect: { fill: "var(--qr-background)" }
                    },
                    target: "#qr-container"
                }
            ],
            { width: "min(90vw, 320px)", height: "auto" }
        );
    } catch (err) {
        console.error("Failed to generate QR code:", err);
    }
}

ws.on("get_show_data", data => {
    const show = data.data;
    showData = show;

    elemShowName.innerText = show.show.name;
    renderCueNotes();
    elemItemCounter.innerText = `${show.lines.length} Items`;

    show.lines.forEach((line, index) => {
        generateDynamic([
            { type: "div", classes: "cue-card", data: { lineId: line.id }, style: { cursor: "pointer" }, events: { click: () => jumpToCue(index + 1) }, varId: "wrapper" },
            { type: "div", classes: "cue-info", target: "@wrapper", varId: "infoSection" },
            { type: "span", classes: "cue-title", text: `Cue ${index + 1}: ${line.name}`, target: "@infoSection" },
            { type: "span", classes: "cue-time", text: `Duration: ${line.duration}s`, target: "@infoSection" },
            { type: "span", classes: "cue-status", text: "pending", style: { color: "var(--text-muted)", fontSize: "0.8rem" }, target: "@wrapper" }
        ], "#cueList");
    });

    show.roles.forEach(role => {
        generateDynamic([
            { type: "div", classes: "role-item", varId: "outerWrapper" },
            { type: "div", classes: "role-details", varId: "wrapper", target: "@outerWrapper" },
            { type: "strong", text: role.name, target: "@wrapper" },
            { type: "span", classes: "role-user", text: "Users: -", data: { roleId: role.id }, target: "@wrapper" }
        ], "#role-list");
    });
});

ws.on("get_session_data", data => {
    sessionData = data.session;
    renderCueNotes();

    sessionData.joined_users.forEach(user => addUserToElem(user.client_role, user.client_name));
});

ws.on("join_session", ({ command: _command, ...user }) => {
    sessionData?.joined_users.push({ ...user, is_caller: false });
    addUserToElem(user.client_role, user.client_name);
});

function addUserToElem(roleId: string | null, userName: string | null): void {
    const elem = document.querySelector<HTMLElement>(`.role-user[data-role-id="${roleId}"]`);
    if (!elem) return;

    if (elem.innerText === "Users: -") {
        elem.innerText = `Users: ${userName}`;
    } else {
        elem.innerText += `, ${userName}`;
    }
}

function renderCueNotes(): void {
    if (!showData) return;

    const lines = [...showData.lines].sort(compareLines);
    const currentCue = sessionData?.current_cue;
    const currentIndex = Number.isInteger(currentCue) && currentCue! > 0 ? currentCue! - 1 : -1;

    renderCueNoteCard("active", lines[currentIndex] || null);
    renderCueNoteCard("next", lines[currentIndex + 1] || null);
}

function renderCueNoteCard(kind: "active" | "next", line: ScenarioLine | null): void {
    const prefix = kind === "active" ? "current" : "next";
    const number = document.getElementById(`${prefix}CueNumber`);
    const title = document.getElementById(`${prefix}CueTitle`);
    const notes = document.getElementById(`${prefix}CueNotes`);
    if (!number || !title || !notes || !showData) return;

    number.textContent = line ? `Cue ${showData.lines.slice().sort(compareLines).findIndex(l => l.id === line.id) + 1}` : "—";
    title.textContent = line ? line.name : kind === "active" ? "Waiting for cue" : "No upcoming cue";
    notes.replaceChildren();

    if (!line) {
        notes.textContent = kind === "active" ? "Waiting for the first cue." : "No upcoming cue.";
        return;
    }

    const lineComment = line.note?.trim();
    if (lineComment) {
        dynamicGenerator({ type: "div", classes: "note-line-comment", text: lineComment }, notes);
    }

    const comments = (showData.comments || [])
        .filter(comment => comment.line_id === line.id);
    const orderedComments = sortCommentsByRole(comments, showData.roles);

    orderedComments.forEach(comment => {
        const roleName = comment.role_id == null
            ? "General"
            : showData?.roles.find(role => role.id === comment.role_id)?.name || "Role";
        dynamicGenerator({
            type: "div",
            classes: "note-role-comment",
            children: [{ type: "strong", text: `${roleName}: ` }, { type: "span", text: comment.comment }]
        }, notes);
    });

    if (!lineComment && comments.length === 0) {
        notes.textContent = "No notes for this cue.";
    }
}

// Saying which cue we saw lets the server ignore a press another caller already made.
btnNextCue.addEventListener("click", () => ws.emit("next_cue", { from_cue: sessionData?.current_cue ?? null }));
btnPreviousCue.addEventListener("click", () => ws.emit("previous_cue", { from_cue: sessionData?.current_cue ?? null }));
btnRelease.addEventListener("click", () => ws.emit("release_caller"));

function jumpToCue(cueNumber: number): void {
    ws.emit("jump_to_cue", { cue_number: cueNumber });
}

//************************
// Timers
// ***********************/

startTimers(document, () => ({ showData, sessionData }));
