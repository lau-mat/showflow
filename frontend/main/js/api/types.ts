import type { Change, Op } from "/shared/js/sync_types";
import type {
    ScenarioLine, ScenarioLineComment, Show, ShowDetails, ShowRole
} from "/shared/js/show_types";

/** Mirrors backend/src/auth.rs `UserView`. */
export interface AuthUser {
    id: string;
    username: string;
    email: string;
    email_verified: boolean;
}

/** Mirrors backend/src/auth.rs `TokenResponse`. */
export interface TokenResponse {
    access_token: string;
    token_type: string;
    expires_in: number;
    /** Absent in browsers: their refresh token is an HttpOnly cookie. */
    refresh_token?: string;
    refresh_expires_at: string;
    user: AuthUser;
}

/** Mirrors backend/src/auth.rs `DeviceView`. */
export interface Device {
    id: string;
    platform: "web" | "desktop" | "mobile";
    app_version: string | null;
    last_seen: string;
    current: boolean;
}

/** The server answered with an error status. */
export class ApiError extends Error {
    readonly status: number;

    constructor(status: number, message: string) {
        super(message);
        this.name = "ApiError";
        this.status = status;
    }
    /** The credentials or token were rejected (as opposed to the server being unavailable). */
    get isRejection(): boolean {
        return this.status >= 400 && this.status < 500;
    }
}

/** The server could not be reached at all. */
export class NetworkError extends Error {
    constructor(message = "Unable to reach the ShowFlow server.") {
        super(message);
        this.name = "NetworkError";
    }
}

/** Cloud identity. Always REST, on every platform. */
export interface AuthApi {
    register(username: string, email: string, password: string): Promise<TokenResponse>;
    login(email: string, password: string): Promise<TokenResponse>;
    /** `null` in browsers: the server reads the refresh cookie instead. */
    refresh(refreshToken: string | null): Promise<TokenResponse>;
    logout(refreshToken: string | null): Promise<void>;
    me(accessToken: string): Promise<{ user_id: string; user: AuthUser }>;
    /** Devices that still hold a login for this account. */
    listDevices(): Promise<Device[]>;
    revokeDevice(deviceId: string): Promise<void>;
    logoutAll(): Promise<void>;
}

export interface NewScenarioLine {
    showId: string;
    /** The line to insert behind; null puts it at the start of the script. */
    after: string | null;
    lineName: string;
    lineNote: string;
    duration: number;
    absoluteTime: number | null;
}

/** The details of a show that can be changed; only the ones present are sent. */
export interface ShowEdit {
    name?: string;
    /** Start time, epoch seconds. */
    time?: number;
}

/** The fields of a cue that can be changed after it exists; only the ones present are sent. */
export interface LineEdit {
    name?: string;
    note?: string;
    duration?: number;
    absoluteTime?: number | null;
}

/** Show content. Tauri IPC (local sqlite) in the apps, REST on the web. */
export interface DataApi {
    /**
     * Called when something other than this page changed show data (another device or person).
     * `showId` is null when the list of shows changed. Returns the unsubscribe function.
     */
    subscribe(listener: (change: { showId: string | null; rev?: number }) => void): () => void;
    getShows(): Promise<Show[]>;
    newShow(name: string, time: number): Promise<Show>;
    editShow(showId: string, changes: ShowEdit): Promise<void>;
    deleteShow(id: string): Promise<void>;
    getShowDetails(showId: string): Promise<ShowDetails>;
    addRole(showId: string, roleName: string): Promise<ShowRole>;
    addLine(line: NewScenarioLine): Promise<ScenarioLine>;
    editLine(showId: string, lineId: string, changes: LineEdit): Promise<void>;
    deleteLine(showId: string, lineId: string): Promise<void>;
    addComment(showId: string, lineId: string, roleId: string | null, comment: string): Promise<ScenarioLineComment>;
    editComment(showId: string, commentId: string, roleId: string | null, comment: string): Promise<void>;
}

/** Hosting a live session (show logic). Only exists where `sessionHost` is available. */
export interface HostApi {
    startSession(showId: string): Promise<string>;
    onSessionStopped(callback: () => void): Promise<() => void>;
}

/** The local side of syncing (apps only): the outbox and the cursors in the app database. */
export interface SyncApi {
    state(): Promise<SyncState>;
    pending(limit: number): Promise<PendingOp[]>;
    ack(showId: string, opIds: string[]): Promise<void>;
    applyChanges(showId: string, changes: Change[], rev: number, role: string | null): Promise<void>;
    forgetShow(showId: string): Promise<void>;
    adoptAccount(userId: string): Promise<number>;
}

export interface SyncShowState {
    show_id: string;
    server_rev: number;
    /** False for a show created here that the cloud has not seen yet. */
    synced: boolean;
    role: string;
}

export interface SyncState {
    account: string | null;
    shows: SyncShowState[];
    pending_total: number;
}

export interface PendingOp {
    show_id: string;
    op: Op;
}

/** Sharing a show (always the cloud, on every platform). Mirrors backend/src/sharing.rs. */
export interface Member {
    user_id: string;
    username: string;
    role: string;
    is_me: boolean;
}

export interface InviteInfo {
    id: string;
    role: string;
    /** Set when only that account may accept. */
    email: string | null;
    expires_at: string;
    max_uses: number;
    used_count: number;
}

export interface Invited {
    /** An existing account that was added straight away. */
    added: { user_id: string; username: string } | null;
    invite: InviteInfo | null;
    /** The secret half of the invite link; shown once. */
    token: string | null;
}

export interface InvitePreview {
    show_id: string;
    show_name: string;
    role: string;
    invited_by: string | null;
    already_member: boolean;
}

export interface NewInvite {
    role: "editor" | "commenter" | "viewer";
    email?: string;
    maxUses?: number;
    expiresInDays?: number;
}

export interface ShareApi {
    members(showId: string): Promise<Member[]>;
    setRole(showId: string, userId: string, role: string): Promise<void>;
    removeMember(showId: string, userId: string): Promise<void>;
    invites(showId: string): Promise<InviteInfo[]>;
    createInvite(showId: string, invite: NewInvite): Promise<Invited>;
    revokeInvite(showId: string, inviteId: string): Promise<void>;
    previewInvite(token: string): Promise<InvitePreview>;
    acceptInvite(token: string): Promise<{ show_id: string; role: string }>;
}
