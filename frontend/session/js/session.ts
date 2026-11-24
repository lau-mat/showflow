import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { addHoverBox, dynamicGenerator, generateDynamic } from "/shared/js/dynamic_generator";
import { SessionSocket } from "/shared/js/session_socket";
import type { SessionState } from "/shared/js/session_types";
import type { ScenarioLine, ShowDetails } from "/shared/js/show_types";
import { startTimers } from "./timers";

const ws = new SessionSocket();

let showData: ShowDetails | null = null;
let sessionData: SessionState | null = null;

function requireElement(id: string): HTMLElement {
    const element = document.getElementById(id);
    if (!element) throw new Error(`Missing element #${id}`);
    return element;
}

const elemShowName = requireElement("ShowName");
const elemItemCounter = requireElement("itemCounter");
const btnNextCue = requireElement("btnNextCue");
const btnPreviousCue = requireElement("previousCue");

function addActionButtons(): void {
    dynamicGenerator({ type: "svg", file: "icons/share.svg", events: { click: share }, classes: ["btn", "icon-btn"], style: { width: "24px", height: "24px" } }, "#actionBtnContainer");
    dynamicGenerator({ type: "svg", file: "icons/fullScreen.svg", events: { click: toggleFullscreen }, classes: ["btn", "icon-btn"], style: { width: "24px", height: "24px", marginLeft: "5px" } }, "#actionBtnContainer");
}

// Module scripts are deferred, so the DOM may already be parsed by now.
if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", addActionButtons);
} else {
    addActionButtons();
}

ws.on("error", data => {
    throw new Error(`Backend error: ${data.message}`);
});

ws.on("cue_change", ({ current_cue, next_cue, show_started_at, cue_started_at }) => {
    if (!showData) return;
    if (sessionData) {
        sessionData.current_cue = current_cue;
        sessionData.show_started_at = show_started_at;
        sessionData.cue_started_at = cue_started_at;
    }
    renderCueNotes();

    // Reset all cards
    document.querySelectorAll(".cue-card").forEach(el => {
        el.classList.remove("active", "next");
        const badge = el.querySelector("span:last-child");
        if (badge) badge.textContent = "pending";
    });

    const sorted = [...showData.lines].sort((a, b) => a.order - b.order);
    const activeId = sorted[current_cue - 1]?.id;
    const nextId = next_cue == null ? undefined : sorted[next_cue - 1]?.id;

    const activeCard = activeId != null ? document.querySelector(`.cue-card[data-line-id="${activeId}"]`) : null;
    if (activeCard) {
        activeCard.classList.add("active");
        const badge = activeCard.querySelector("span:last-child");
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
        const qrSvg = await invoke<string>("generate_server_qr");

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

    show.lines.forEach(line => {
        generateDynamic([
            { type: "div", classes: "cue-card", data: { lineId: line.id }, style: { cursor: "pointer" }, events: { click: () => jumpToCue(line.id) }, varId: "wrapper" },
            { type: "div", classes: "cue-info", target: "@wrapper", varId: "infoSection" },
            { type: "span", classes: "cue-title", text: `Cue ${line.id}: ${line.name}`, target: "@infoSection" },
            { type: "span", classes: "cue-time", text: `Duration: ${line.duration}s`, target: "@infoSection" },
            { type: "span", text: "pending", style: { color: "var(--text-muted)", fontSize: "0.8rem" }, target: "@wrapper" }
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
    sessionData?.joined_users.push(user);
    addUserToElem(user.client_role, user.client_name);
});

function addUserToElem(roleId: number | null, userName: string | null): void {
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

    const lines = [...showData.lines].sort((a, b) => a.order - b.order);
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

    number.textContent = line ? `Cue ${line.id}` : "—";
    title.textContent = line ? line.name : kind === "active" ? "Waiting for cue" : "No upcoming cue";
    notes.replaceChildren();

    if (!line) {
        notes.textContent = kind === "active" ? "Waiting for the first cue." : "No upcoming cue.";
        return;
    }

    const lineComment = line.comment?.trim();
    if (lineComment) {
        dynamicGenerator({ type: "div", classes: "note-line-comment", text: lineComment }, notes);
    }

    const comments = (showData.comments || [])
        .filter(comment => comment.line_id === line.id)
        .sort((a, b) => (a.role_id ?? -1) - (b.role_id ?? -1));

    comments.forEach(comment => {
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

btnNextCue.addEventListener("click", () => ws.emit("next_cue"));
btnPreviousCue.addEventListener("click", () => ws.emit("previous_cue"));

function jumpToCue(cueId: number): void {
    ws.emit("jump_to_cue", { cue_number: cueId });
}

//************************
// Timers
// ***********************/

startTimers(document, () => ({ showData, sessionData }));
