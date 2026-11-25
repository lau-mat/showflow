import { getPlatform, type PlatformProfile } from "/shared/js/platform";
import { AuthSession } from "../auth/session";
import { createIpcTokenStore, createLocalTokenStore, type TokenStore } from "../auth/token_store";
import { SyncEngine } from "../sync/engine";
import { LiveUpdates } from "../sync/live";
import { ChangeBus } from "../sync/bus";
import { SyncStatusStore } from "../sync/status";
import { refreshCallerAccess } from "../sync/caller_access";
import { createIpcCallerApi, createIpcDataApi, createIpcDiscoveryApi, createIpcHostApi, createIpcSyncApi } from "./ipc";
import { createRestAuthApi, createRestDataApi, createRestShareApi, RestClient } from "./rest";
import type { AuthApi, DataApi, DiscoveryApi, HostApi, ShareApi } from "./types";

export interface ApiBundle {
    auth: AuthApi;
    share: ShareApi;
    session: AuthSession;
    data: DataApi;
    /** Null where the platform cannot host sessions. */
    host: HostApi | null;
    /** Null where the platform cannot search the network (a browser). */
    discovery: DiscoveryApi | null;
    /** Keeps the local database converged with the cloud; null on the web (no local copy). */
    sync: SyncEngine | null;
    syncStatus: SyncStatusStore;
    bus: ChangeBus;
    /** The server-sent-events connection that announces changes made elsewhere. */
    live: LiveUpdates;
}

let installed: ApiBundle | null = null;

export function installApi(bundle: ApiBundle | null): void {
    installed = bundle;
}

export function getApi(): ApiBundle {
    if (!installed) throw new Error("API not initialised yet; await initApi() first.");
    return installed;
}

const env = (import.meta as { env?: Record<string, string | undefined> }).env;

/** `/api` through the Vite/nginx proxy on the web (and in dev); an absolute cloud url in app builds. */
export function cloudBaseUrl(profile: PlatformProfile): string {
    if (profile.transport === "rest") return "/api";
    return env?.VITE_CLOUD_URL ?? "/api";
}

/** Picks the implementations for the detected platform. Call after `detectPlatform()`. */
export async function initApi(profile: PlatformProfile = getPlatform()): Promise<ApiBundle> {
    const store: TokenStore = profile.transport === "ipc" ? createIpcTokenStore() : createLocalTokenStore();
    const { deviceId } = await store.load();

    const clientHeaders = () => ({
        "X-ShowFlow-Platform": `${profile.kind}/${profile.os}`,
        "X-ShowFlow-Device-Id": deviceId,
        "X-ShowFlow-App-Version": env?.VITE_APP_VERSION ?? "dev"
    });

    // The session needs the REST auth api, and the client needs the session to refresh tokens.
    let session: AuthSession | null = null;
    const client = new RestClient({
        baseUrl: cloudBaseUrl(profile),
        clientHeaders,
        getAccessToken: () => session!.getAccessToken(),
        refreshAccessToken: () => session!.forceRefresh(),
        credentials: profile.tokenStorage === "cookie" ? "same-origin" : "omit"
    });
    const auth = createRestAuthApi(client);
    session = new AuthSession({
        api: auth,
        store,
        offlineGrace: profile.offlineGraceDays !== null,
        cookieMode: profile.tokenStorage === "cookie"
    });

    const bus = new ChangeBus();
    const syncStatus = new SyncStatusStore();
    const local = profile.transport === "ipc";
    const callers = createIpcCallerApi();
    const sync = local ? new SyncEngine({
        session, rest: client, local: createIpcSyncApi(), bus, status: syncStatus,
        afterPull: index => refreshCallerAccess(index, { rest: client, local: callers })
    }) : null;
    const live = new LiveUpdates({
        open: signal => client.stream("sync/stream", signal),
        session,
        onEvent: event => {
            // Apps pull into their local database; the web has nothing local, so pages re-read.
            if (sync) return sync.trigger(0);
            if (event.type === "show") bus.emitExternal(event.showId, event.rev);
            else bus.emitExternal(null);
        }
    });
    const bundle: ApiBundle = {
        auth,
        share: createRestShareApi(client),
        session,
        data: local ? createIpcDataApi(bus) : createRestDataApi(client, bus),
        host: profile.features.sessionHost ? createIpcHostApi() : null,
        discovery: profile.features.sessionDiscovery ? createIpcDiscoveryApi() : null,
        sync,
        syncStatus,
        bus,
        live
    };
    installApi(bundle);
    return bundle;
}
