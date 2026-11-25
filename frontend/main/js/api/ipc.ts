import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ChangeBus } from "../sync/bus";
import type { DataApi, HostApi, SyncApi } from "./types";

/** Local-first show data: every edit lands in the app database at once and is synced later. */
export function createIpcDataApi(bus: ChangeBus): DataApi {
    // An edit is only worth a sync once it has actually been stored.
    const written = <T>(call: Promise<T>): Promise<T> => call.then(result => { bus.emitLocalWrite(); return result; });
    return {
        subscribe: listener => bus.subscribe(listener),
        getShows: () => invoke("get_shows"),
        newShow: (name, time) => written(invoke("new_show", { name, time })),
        deleteShow: id => written(invoke("delete_show", { id })),
        getShowDetails: showId => invoke("get_full_show_details", { showId }),
        addRole: (showId, roleName) => written(invoke("add_role", { showId, roleName })),
        addLine: line => written(invoke("add_scenario_line", { ...line })),
        // The local store finds the show from the line/comment itself, so `showId` is not sent.
        deleteLine: (_showId, lineId) => written(invoke("delete_scenario_line", { lineId })),
        addComment: (_showId, lineId, roleId, comment) => written(invoke("add_scenario_line_comment", { lineId, roleId, comment })),
        editComment: (_showId, commentId, roleId, comment) => written(invoke("edit_scenario_line_comment", { commentId, roleId, comment }))
    };
}

export function createIpcSyncApi(): SyncApi {
    return {
        state: () => invoke("sync_state"),
        pending: limit => invoke("sync_pending", { limit }),
        ack: (showId, opIds) => invoke("sync_ack", { showId, opIds }),
        applyChanges: (showId, changes, rev, role) => invoke("sync_apply_changes", { showId, changes, rev, role }),
        forgetShow: showId => invoke("sync_forget_show", { showId }),
        adoptAccount: userId => invoke("sync_adopt_account", { userId })
    };
}

export function createIpcHostApi(): HostApi {
    return {
        startSession: showId => invoke("start_session", { showId }),
        onSessionStopped: callback => listen("session-stopped", callback)
    };
}
