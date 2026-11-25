import { invoke } from "@tauri-apps/api/core";

export interface StoredTokens {
    /** Apps only; browsers keep theirs in an HttpOnly cookie that JS cannot read. */
    refreshToken: string | null;
    /** A login probably exists, so restoring is worth a request (always known for apps). */
    hasSession: boolean;
    refreshExpiresAt: string | null;
    deviceId: string;
    /** Seconds of offline use left; null where there is no offline grace (web). */
    graceRemainingSeconds: number | null;
}

export interface TokenStore {
    load(): Promise<StoredTokens>;
    /** `refreshToken` is null in cookie mode: only the "logged in" hint is remembered. */
    save(refreshToken: string | null, refreshExpiresAt: string, userId?: string | null): Promise<void>;
    clear(): Promise<void>;
}

interface NativeStoredAuth {
    refresh_token: string | null;
    refresh_expires_at: string | null;
    device_id: string;
    grace_remaining_seconds: number;
}

/** Apps: the local sqlite db, through the Rust `auth_*` commands. */
export function createIpcTokenStore(): TokenStore {
    return {
        async load() {
            const stored = await invoke<NativeStoredAuth>("auth_load");
            return {
                refreshToken: stored.refresh_token,
                hasSession: stored.refresh_token !== null,
                refreshExpiresAt: stored.refresh_expires_at,
                deviceId: stored.device_id,
                graceRemainingSeconds: stored.grace_remaining_seconds
            };
        },
        async save(refreshToken, refreshExpiresAt, userId = null) {
            if (refreshToken === null) throw new Error("Apps must store the refresh token.");
            await invoke("auth_save", { refreshToken, refreshExpiresAt, userId });
        },
        async clear() {
            await invoke("auth_clear");
        }
    };
}

const SESSION_HINT_KEY = "showflow.session_expires_at";
const DEVICE_KEY = "showflow.device_id";

/**
 * Web: the refresh token is an HttpOnly cookie, so this only keeps the device id and a hint
 * (no secret) that a login exists, which avoids a pointless refresh request on the login page.
 */
export function createLocalTokenStore(storage: Pick<Storage, "getItem" | "setItem" | "removeItem"> = window.localStorage): TokenStore {
    const deviceId = () => {
        let id = storage.getItem(DEVICE_KEY);
        if (!id) {
            id = crypto.randomUUID();
            storage.setItem(DEVICE_KEY, id);
        }
        return id;
    };
    return {
        async load() {
            const hint = storage.getItem(SESSION_HINT_KEY);
            return {
                refreshToken: null,
                hasSession: hint !== null,
                refreshExpiresAt: hint,
                deviceId: deviceId(),
                graceRemainingSeconds: null
            };
        },
        async save(_refreshToken, refreshExpiresAt) {
            storage.setItem(SESSION_HINT_KEY, refreshExpiresAt);
        },
        async clear() {
            storage.removeItem(SESSION_HINT_KEY);
        }
    };
}
