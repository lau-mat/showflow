import { newOp, type EntityKind, type Op, type OpResult, type Place, type PushResponse } from "/shared/js/sync_types";
import type { Show, ShowDetails } from "/shared/js/show_types";
import type { ChangeBus } from "../sync/bus";
import { ApiError, NetworkError, type AuthApi, type AuthUser, type DataApi, type Device, type InviteInfo, type InvitePreview, type Invited, type Member, type ShareApi, type TokenResponse } from "./types";

export interface RestClientOptions {
    /** `/api` on the web (proxied); an absolute cloud url in app builds. */
    baseUrl: string;
    /** Platform / device / version headers attached to every request (licence monitoring). */
    clientHeaders: () => Record<string, string>;
    /** Supplies a valid access token for authorized requests. */
    getAccessToken?: () => Promise<string>;
    /** Called once after a 401 to obtain a fresh access token for a retry. */
    refreshAccessToken?: () => Promise<string>;
    /** Browsers send their same-origin refresh cookie; apps never use cookies. */
    credentials: "same-origin" | "omit";
}

export interface RequestOptions {
    method?: "GET" | "POST" | "PATCH" | "DELETE";
    body?: unknown;
    /** Aborts the request (used to close the live stream). */
    signal?: AbortSignal;
    /** Attach a Bearer token; retries once through `refreshAccessToken` on 401. */
    authorized?: boolean;
    token?: string;
}

export class RestClient {
    private readonly options: RestClientOptions;

    constructor(options: RestClientOptions) {
        this.options = options;
    }

    async request<T>(path: string, request: RequestOptions = {}): Promise<T> {
        let token = request.token;
        if (request.authorized && token === undefined) token = await this.options.getAccessToken?.();

        let response = await this.send(path, request, token);
        if (response.status === 401 && request.authorized && this.options.refreshAccessToken) {
            response = await this.send(path, request, await this.options.refreshAccessToken());
        }

        if (!response.ok) {
            const body = await response.json().catch(() => null) as { error?: string } | null;
            throw new ApiError(response.status, body?.error ?? response.statusText);
        }
        if (response.status === 204) return undefined as T;
        return await response.json() as T;
    }

    /** Opens a long-lived response (server-sent events) with the signed-in user's token. */
    async stream(path: string, signal: AbortSignal): Promise<Response> {
        const token = await this.options.getAccessToken?.();
        let response = await this.send(path, { signal }, token);
        if (response.status === 401 && this.options.refreshAccessToken) {
            response = await this.send(path, { signal }, await this.options.refreshAccessToken());
        }
        if (!response.ok) {
            const body = await response.json().catch(() => null) as { error?: string } | null;
            throw new ApiError(response.status, body?.error ?? response.statusText);
        }
        return response;
    }

    private async send(path: string, request: RequestOptions, token: string | undefined): Promise<Response> {
        const headers: Record<string, string> = { ...this.options.clientHeaders() };
        if (request.body !== undefined) headers["Content-Type"] = "application/json";
        if (token) headers.Authorization = `Bearer ${token}`;
        try {
            return await fetch(`${this.options.baseUrl}/${path}`, {
                method: request.method ?? (request.body === undefined ? "GET" : "POST"),
                headers,
                signal: request.signal,
                credentials: this.options.credentials,
                body: request.body === undefined ? undefined : JSON.stringify(request.body)
            });
        } catch {
            throw new NetworkError();
        }
    }
}

export function createRestAuthApi(client: RestClient): AuthApi {
    // Browsers pass no token: the body stays empty and the refresh cookie travels instead.
    const tokenBody = (refreshToken: string | null) =>
        refreshToken === null ? {} : { body: { refresh_token: refreshToken } };
    return {
        register: (username, email, password) =>
            client.request<TokenResponse>("auth/register", { body: { username, email, password } }),
        login: (email, password) =>
            client.request<TokenResponse>("auth/login", { body: { email, password } }),
        refresh: refreshToken =>
            client.request<TokenResponse>("auth/refresh", { method: "POST", ...tokenBody(refreshToken) }),
        logout: refreshToken =>
            client.request<void>("auth/logout", { method: "POST", ...tokenBody(refreshToken) }),
        me: accessToken =>
            client.request<{ user_id: string; user: AuthUser }>("auth/me", { token: accessToken }),
        listDevices: () =>
            client.request<Device[]>("auth/devices", { authorized: true }),
        revokeDevice: deviceId =>
            client.request<void>(`auth/devices/${encodeURIComponent(deviceId)}`, { method: "DELETE", authorized: true }),
        logoutAll: () =>
            client.request<void>("auth/logout-all", { method: "POST", authorized: true })
    };
}

/**
 * Show content over REST (the web). Reads fetch snapshots; every edit is one op pushed to
 * `POST /shows/:id/ops`, the same write path the apps use when they sync.
 */
export function createRestDataApi(client: RestClient, bus: ChangeBus): DataApi {
    const pushOps = async (showId: string, ops: Op[]): Promise<OpResult[]> => {
        const { results } = await client.request<PushResponse>(`shows/${showId}/ops`, { body: { ops }, authorized: true });
        for (const result of results) {
            if (result.status === "rejected") throw new ApiError(422, result.error);
        }
        return results;
    };
    const setField = (showId: string, entity: EntityKind, id: string, fields: Record<string, unknown>, place?: Place) =>
        pushOps(showId, [newOp(entity, id, "set", fields, place)]);

    return {
        subscribe: listener => bus.subscribe(listener),
        getShows: () => client.request<Show[]>("shows", { authorized: true }),
        async newShow(name, time) {
            const id = crypto.randomUUID();
            await setField(id, "show", id, { name, time });
            return { id, name, time, role: "owner" };
        },
        async editShow(showId, changes) {
            await setField(showId, "show", showId, { ...changes });
        },
        async deleteShow(id) {
            await pushOps(id, [newOp("show", id, "delete")]);
        },
        getShowDetails: showId => client.request<ShowDetails>(`shows/${showId}`, { authorized: true }),
        async addRole(showId, roleName) {
            const id = crypto.randomUUID();
            await setField(showId, "role", id, { name: roleName });
            return { id, show_id: showId, name: roleName };
        },
        async addLine(line) {
            const id = crypto.randomUUID();
            const [result] = await setField(
                line.showId, "line", id,
                { name: line.lineName, note: line.lineNote, duration: line.duration, absolute_time: line.absoluteTime },
                { after: line.after }
            );
            return {
                id,
                position: result.status === "applied" ? result.position ?? "" : "",
                name: line.lineName,
                note: line.lineNote,
                duration: line.duration,
                absolute_time: line.absoluteTime
            };
        },
        async editLine(showId, lineId, changes) {
            const { absoluteTime, ...rest } = changes;
            await setField(showId, "line", lineId, { ...rest, ...(absoluteTime !== undefined ? { absolute_time: absoluteTime } : {}) });
        },
        async deleteLine(showId, lineId) {
            await pushOps(showId, [newOp("line", lineId, "delete")]);
        },
        async addComment(showId, lineId, roleId, comment) {
            const id = crypto.randomUUID();
            await setField(showId, "comment", id, { line_id: lineId, role_id: roleId, comment });
            return { id, line_id: lineId, role_id: roleId, comment };
        },
        async editComment(showId, commentId, roleId, comment) {
            await setField(showId, "comment", commentId, { role_id: roleId, comment });
        }
    };
}

export function createRestShareApi(client: RestClient): ShareApi {
    const show = (showId: string) => `shows/${showId}`;
    return {
        members: showId => client.request<Member[]>(`${show(showId)}/members`, { authorized: true }),
        setRole: (showId, userId, role) =>
            client.request<void>(`${show(showId)}/members/${userId}`, { method: "PATCH", body: { role }, authorized: true }),
        removeMember: (showId, userId) =>
            client.request<void>(`${show(showId)}/members/${userId}`, { method: "DELETE", authorized: true }),
        invites: showId => client.request<InviteInfo[]>(`${show(showId)}/invites`, { authorized: true }),
        createInvite: (showId, invite) => client.request<Invited>(`${show(showId)}/invites`, {
            body: { role: invite.role, email: invite.email, max_uses: invite.maxUses, expires_in_days: invite.expiresInDays },
            authorized: true
        }),
        revokeInvite: (showId, inviteId) =>
            client.request<void>(`${show(showId)}/invites/${inviteId}`, { method: "DELETE", authorized: true }),
        previewInvite: token => client.request<InvitePreview>("invites/preview", { body: { token }, authorized: true }),
        acceptInvite: token => client.request<{ show_id: string; role: string }>("invites/accept", { body: { token }, authorized: true })
    };
}
