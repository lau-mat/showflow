import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ChangeBus } from "../sync/bus";
import type { CallerStoreApi, DataApi, DiscoveryApi, HostApi, LineEdit, SyncApi } from "./types";

/** The API's camelCase edit as the op fields the store expects. */
function lineFields(changes: LineEdit): Record<string, unknown> {
    const { absoluteTime, ...rest } = changes;
    return { ...rest, ...(absoluteTime !== undefined ? { absolute_time: absoluteTime } : {}) };
}

/** Local-first show data: every edit lands in the app database at once and is synced later. */
export function createIpcDataApi(bus: ChangeBus): DataApi {
    // An edit is only worth a sync once it has actually been stored.
    const written = <T>(call: Promise<T>): Promise<T> => call.then(result => { bus.emitLocalWrite(); return result; });
    return {
        subscribe: listener => bus.subscribe(listener),
        getShows: () => invoke("get_shows"),
        newShow: (name, time) => written(invoke("new_show", { name, time })),
        editShow: (showId, changes) => written(invoke("edit_show", { showId, fields: changes })),
        deleteShow: id => written(invoke("delete_show", { id })),
        getShowDetails: showId => invoke("get_full_show_details", { showId }),
        addRole: (showId, roleName) => written(invoke("add_role", { showId, roleName })),
        addLine: line => written(invoke("add_scenario_line", { ...line })),
        editLine: (_showId, lineId, changes) => written(invoke("edit_scenario_line", { lineId, fields: lineFields(changes) })),
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

export function createIpcCallerApi(): CallerStoreApi {
    return {
        state: () => invoke("caller_state"),
        savePass: (showId, pass, expiresAt) => invoke("caller_save_pass", { showId, pass, expiresAt }),
        saveTrust: (publicKey, lists) => invoke("caller_save_trust", { publicKey, lists })
    };
}

export function createIpcHostApi(): HostApi {
    return {
        startSession: showId => invoke("start_session", { showId }),
        listSessions: () => invoke("list_sessions"),
        status: sessionId => invoke("session_status", { sessionId }),
        stopSession: sessionId => invoke("stop_session", { sessionId }),
        setCode: (sessionId, kind, code, length) => invoke("set_access_code", { sessionId, kind, code, length }),
        answerRequest: (sessionId, clientId, kind, approve) => invoke("answer_access_request", { sessionId, clientId, kind, approve }),
        removeDevice: (sessionId, clientId) => invoke("remove_device", { sessionId, clientId }),
        revokeCaller: (sessionId, grantId) => invoke("revoke_caller", { sessionId, grantId }),
        openCallerWindow: sessionId => invoke("open_caller_window", { sessionId }),
        qr: sessionId => invoke("session_qr", { sessionId }),
        settings: showId => invoke("session_settings", { showId }),
        saveSettings: (showId, settings) => invoke("save_session_settings", { showId, settings }),
        onSessionUpdated: callback => listen<string>("session-updated", event => callback(event.payload))
    };
}

export function createIpcDiscoveryApi(): DiscoveryApi {
    return { find: () => invoke("discover_sessions") };
}
