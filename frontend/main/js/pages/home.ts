import { dynamicPrompt, generateDynamic } from "/shared/js/dynamic_generator";
import "/shared/js/compound_components";
import { hasFeature } from "/shared/js/platform";
import type { Show } from "/shared/js/show_types";
import { getApi } from "../api";
import { openEditShow } from "./edit_show.page";
import { renderDiscovery } from "./join_session";
import { openSessionPanel, startShowSession } from "./session_panel";

/** show id -> id of its running session on this computer. */
const runningSessions = new Map<string, string>();

/** Lights up the start icon of every show that has a running session. */
async function markRunningSessions(): Promise<void> {
    const { host } = getApi();
    if (!host) return;
    const sessions = await host.listSessions();
    runningSessions.clear();
    for (const session of sessions) runningSessions.set(session.show_id, session.session_id);
    document.querySelectorAll<HTMLElement>("#editShowsTableBody tr[data-show-id]").forEach(row => {
        row.querySelector(".start-session")?.classList.toggle("session-active", runningSessions.has(row.dataset.showId ?? ""));
    });
}

/** Shows whose start button is being handled: a double click must not start twice. */
const starting = new Set<string>();

/** Starts a session for the show, or opens the panel of the one already running. */
async function startOrManageSession(show: Show): Promise<void> {
    if (starting.has(show.id)) return;
    starting.add(show.id);
    try {
        await markRunningSessions();
        const sessionId = runningSessions.get(show.id) ?? await startShowSession(show);
        if (!sessionId) return;
        await markRunningSessions();
        await openSessionPanel(sessionId);
    } finally {
        starting.delete(show.id);
    }
}

function addShowItem(show: Show): void {
    const { data, host } = getApi();
    const canHost = hasFeature("sessionHost") && host !== null;

    const item = generateDynamic([
        { type: "tr", varId: "row", data: { showId: show.id } },
        { type: "td", varId: "nameCell", target: "@row", text: show.name },
        ...(show.role !== "owner" ? [{ type: "span", classes: "role-tag", text: `shared · ${show.role}`, target: "@nameCell" }] : []),
        { type: "td", text: new Date(show.time * 1000).toLocaleString(), target: "@row" },
        { type: "td", varId: "actionContainer", target: "@row" },
        { type: "div", classes: "actions-cell", target: "@actionContainer", varId: "actionCell" },
        { type: "svg", file: "icons/edit.svg", classes: ["btn-icon", "btn-muted"], target: "@actionCell", varId: "editBtn" },
        ...(canHost
            ? [{ type: "svg", file: "icons/start.svg", classes: ["btn-icon", "btn-muted", "start-session", ...(runningSessions.has(show.id) ? ["session-active"] : [])], attributes: { title: "Session" }, target: "@actionCell", varId: "startSession" }]
            : []),
        { type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], attributes: { title: show.role === "owner" ? "Delete this show" : "Leave this show" }, target: "@actionCell", varId: "delBtn" }
    ], "#editShowsTableBody");

    item.delBtn.addEventListener("click", async () => {
        if (show.role === "owner") {
            await data.deleteShow(show.id);
        } else {
            // Not yours to delete: leaving removes only your own access.
            const { share, session, sync } = getApi();
            await share.removeMember(show.id, session.user?.id ?? "");
            sync?.trigger(0);
        }
        item.row.remove();
    });
    item.editBtn.addEventListener("click", () => openEditShow(show.id));
    if (canHost) {
        item.startSession.addEventListener("click", () => { void startOrManageSession(show); });
    }
}

async function createShow(): Promise<void> {
    const newShowData = await dynamicPrompt({
        title: "New Show Details",
        confirmText: "Create",
        elements: [
            { type: "input-label", id: "newShowName", label: "Show name" },
            { type: "datetime-label", id: "newShowTime", label: "Show time" }
        ],
        onValidate: data => {
            const errors: Record<string, string> = {};
            if (!data.newShowName) errors.newShowName = "Please fill in a name";
            if (!Number(data.newShowTime)) errors.newShowTime = "A start time must be selected";
            return errors;
        }
    });
    if (!newShowData) return;

    const time = Number(newShowData.newShowTime);
    addShowItem(await getApi().data.newShow(String(newShowData.newShowName), time));
}

export async function loadHomePage(): Promise<void> {
    const { data, host } = getApi();

    if (hasFeature("sessionDiscovery")) renderDiscovery();

    generateDynamic([
        { type: "div", id: "editShows" },
        { type: "table", target: "#editShows", id: "editShowsTable" },
        { type: "thead", target: "#editShowsTable", varId: "editShowsTableHead" },
        { type: "tr", target: "@editShowsTableHead", varId: "editShowsHeaderRow" },
        { type: "th", target: "@editShowsHeaderRow", text: "Show Name" },
        { type: "th", target: "@editShowsHeaderRow", text: "Show Time" },
        { type: "th", target: "@editShowsHeaderRow", text: "Actions" },
        { type: "tbody", target: "#editShowsTable", id: "editShowsTableBody" },
        { type: "tfoot", target: "#editShowsTable", varId: "editShowsTableFoot" },
        { type: "tr", target: "@editShowsTableFoot", varId: "editShowsFooterRow" },
        { type: "td", target: "@editShowsFooterRow", classes: "create-show-cell", attributes: { colspan: "3" }, varId: "editShowsFooterCell" },
        {
            type: "button", target: "@editShowsFooterCell", id: "createShowBtn",
            events: { click: createShow },
            children: [
                { type: "span", classes: "plus-icon", text: "+" },
                { type: "span", text: "Create Show" }
            ]
        }
    ], "#mainSection");

    (await data.getShows()).forEach(addShowItem);

    // Shows created, shared or removed elsewhere appear and disappear here without a reload.
    const stopWatching = data.subscribe(async change => {
        const body = document.getElementById("editShowsTableBody");
        if (!body) return stopWatching(); // the user left this page
        if (change.showId !== null && !body.querySelector(`[data-show-id="${change.showId}"]`)) return;
        const shows = await data.getShows();
        body.replaceChildren();
        shows.forEach(addShowItem);
    });

    // Keep the "running" markers right as sessions start and end.
    if (host && hasFeature("sessionHost")) {
        await markRunningSessions();
        const stop = await host.onSessionUpdated(() => {
            if (!document.getElementById("editShowsTableBody")) return stop(); // the user left this page
            void markRunningSessions();
        });
    }
}
