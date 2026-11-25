import { dynamicPrompt, generateDynamic } from "/shared/js/dynamic_generator";
import "/shared/js/compound_components";
import { hasFeature } from "/shared/js/platform";
import type { Show } from "/shared/js/show_types";
import { getApi } from "../api";
import { openEditShow } from "./edit_show.page";

/** Table of sessions found on the network (join targets). */
export function renderSessionDiscovery(): void {
    generateDynamic([
        { type: "div", id: "sessionDiscovery" },
        { type: "table", target: "#sessionDiscovery", id: "discoveredSessions" },
        { type: "thead", target: "#discoveredSessions", varId: "discoveredSessionsTableHead" },
        { type: "tr", target: "@discoveredSessionsTableHead", varId: "discoveryHeaderRow" },
        { type: "th", target: "@discoveryHeaderRow", text: "Session Name" },
        { type: "th", target: "@discoveryHeaderRow", text: "Session ID" },
        { type: "th", target: "@discoveryHeaderRow", text: "Actions" },
        { type: "tbody", target: "#discoveredSessions", id: "discoveredSessionsTableBody" },
        { type: "tr", target: "#discoveredSessionsTableBody", varId: "noSessionsRow" },
        { type: "td", target: "@noSessionsRow", text: "No sessions discovered yet.", attributes: { colspan: "3" } }
    ], "#mainSection");
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
            ? [{ type: "svg", file: "icons/start.svg", classes: ["btn-icon", "btn-muted"], target: "@actionCell", varId: "startSession" }]
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
        item.startSession.addEventListener("click", async () => {
            await host.startSession(show.id);
            item.startSession.classList.add("session-active");
        });
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

    if (hasFeature("sessionDiscovery")) renderSessionDiscovery();

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

    // Clear the "running" marker when the session window is closed.
    await host?.onSessionStopped(() => {
        document.querySelector("svg.btn-icon.session-active")?.classList.remove("session-active");
    });
}
