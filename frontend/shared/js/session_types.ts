// Mirrors the live-session wire protocol in protocol/src/session.rs.
import type { ShowDetails } from "./show_types";

/** Bumped when a change would make an older page misbehave. */
export const PROTOCOL_VERSION = 2;

export type AccessMethod = "open" | "code" | "request" | "account" | "host";
/** What a host guards: getting in at all (seeing the show), and running it. */
export type AccessKind = "view" | "call";
export type AccessState = "granted" | "pending" | "denied" | "revoked" | "released";

/** Which ways through a gate the host enabled; `open` makes the others meaningless. */
export interface AccessPolicy {
    open: boolean;
    code: boolean;
    request: boolean;
    account: boolean;
}

/** Who may join and call one show's sessions, as the host set it up. */
export interface AccessSettings {
    caller: AccessPolicy;
    caller_code: string | null;
    view: AccessPolicy;
    view_code: string | null;
}

export type Claim =
    | { method: "open"; name: string }
    | { method: "code"; name: string; code: string }
    | { method: "request"; name: string }
    | { method: "account"; pass: string; proof: string }
    | { method: "resume"; token: string }
    | { method: "host"; token: string };

export interface CallerInfo {
    name: string;
    method: AccessMethod;
}

export interface SessionClient {
    client_id: string;
    client_name: string | null;
    client_role: string | null;
    is_caller: boolean;
}

export interface SessionState {
    session_id: string;
    show_id: string;
    is_active: boolean;
    current_cue: number | null;
    show_started_at: number | null;
    cue_started_at: number | null;
    joined_users: SessionClient[];
}

export interface Welcome {
    protocol: number;
    session_id: string;
    show_id: string;
    nonce: string;
    /** Who may become caller. */
    policy: AccessPolicy;
    /** Who may join. When open, this device is in already. */
    view_policy: AccessPolicy;
}

/** Messages the server sends, keyed by their `command` tag (ServerMessage). */
export interface ServerMessages {
    welcome: Welcome;
    policy: { policy: AccessPolicy; view_policy: AccessPolicy };
    /** Whether this device is in; the show follows `granted` by itself. */
    view_status: { state: AccessState; reason: string | null; resume_token: string | null };
    caller_status: { state: AccessState; reason: string | null; resume_token: string | null };
    callers: { list: CallerInfo[] };
    join_session: { client_id: string; client_name: string; client_role: string };
    get_show_data: { data: ShowDetails };
    get_session_data: { session: SessionState };
    cue_change: {
        current_cue: number;
        next_cue: number | null;
        show_started_at: number | null;
        cue_started_at: number | null;
        /** The caller who moved the show (null when only bringing this device up to date). */
        by: string | null;
    };
    error: { message: string };
}

/** Messages a client sends, keyed by their `command` tag (ClientMessage). */
export interface ClientMessages {
    join_session: { user_name: string; role_id: string | null };
    get_show_data: Record<string, never>;
    get_session_data: Record<string, never>;
    request_view: { claim: Claim };
    claim_caller: { claim: Claim };
    release_caller: Record<string, never>;
    /** `from_cue` is what this caller was looking at; a stale press is ignored by the server. */
    next_cue: { from_cue: number | null };
    previous_cue: { from_cue: number | null };
    jump_to_cue: { cue_number: number };
}
