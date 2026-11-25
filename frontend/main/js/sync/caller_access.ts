// Account-based access to live sessions, kept ready for a show without internet. After each sync,
// while online: this computer learns how to check passes (the cloud's key, and per show who is a
// member and who may call), and renews its own pass for every show this account has access to.
import { ApiError, type CallerStoreApi, type ShowAccess } from "../api/types";
import type { SyncTransport } from "./engine";

/** Passes last two weeks; renewing in the second half leaves a week of margin to work offline. */
export const RENEW_WHEN_LEFT_SECONDS = 7 * 24 * 60 * 60;

export interface CallerIndexEntry {
    id: string;
    members?: string[];
    callers?: string[];
}

export interface CallerAccessDeps {
    rest: SyncTransport;
    local: CallerStoreApi;
    now?: () => number;
}

const hasStatus = (error: unknown, ...statuses: number[]) => error instanceof ApiError && statuses.includes(error.status);

export async function refreshCallerAccess(index: CallerIndexEntry[], { rest, local, now = Date.now }: CallerAccessDeps): Promise<void> {
    let publicKey: string;
    try {
        ({ public_key: publicKey } = await rest.request<{ public_key: string }>("caller-passes/key", { authorized: true }));
    } catch (error) {
        if (hasStatus(error, 404)) return; // this server does not offer account access
        throw error;
    }
    const lists: Record<string, ShowAccess> = Object.fromEntries(index.flatMap(
        show => (show.members && show.callers ? [[show.id, { members: show.members, callers: show.callers }]] : [])
    ));
    await local.saveTrust(publicKey, lists);

    // Passes of shows that went away are dropped together with the show.
    const { device_key, passes } = await local.state();
    const expiry = new Map(passes.map(pass => [pass.show_id, pass.expires_at]));
    const renewBefore = now() / 1000 + RENEW_WHEN_LEFT_SECONDS;
    for (const show of index) {
        const heldUntil = expiry.get(show.id);
        if (heldUntil !== undefined && heldUntil > renewBefore) continue;
        try {
            const issued = await rest.request<{ pass: string; expires_at: number }>(`shows/${show.id}/caller-pass`, { body: { device_key }, authorized: true });
            await local.savePass(show.id, issued.pass, issued.expires_at);
        } catch (error) {
            // The show (or the access to it) went away since the list was fetched; the next sync tidies up.
            if (!hasStatus(error, 403, 404)) throw error;
        }
    }
}
