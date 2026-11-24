// Mirrors the live-session structs and websocket messages in app/src/session.rs.
import type { ShowDetails } from "./show_types";

export interface SessionClient {
    client_id: string;
    client_name: string | null;
    client_role: number | null;
}

export interface SessionState {
    session_id: string;
    show_id: number;
    is_active: boolean;
    current_cue: number | null;
    show_started_at: number | null;
    cue_started_at: number | null;
    joined_users: SessionClient[];
}

/** Messages the server sends, keyed by their `command` tag (ServerMessage). */
export interface ServerMessages {
    join_session: { client_id: string; client_name: string; client_role: number };
    get_show_data: { data: ShowDetails };
    get_session_data: { session: SessionState };
    cue_change: {
        current_cue: number;
        next_cue: number | null;
        show_started_at: number | null;
        cue_started_at: number | null;
    };
    error: { message: string };
}

/** Messages a client sends, keyed by their `command` tag (ClientMessage). */
export interface ClientMessages {
    join_session: { user_name: string; role_id: number | null };
    get_show_data: Record<string, never>;
    get_session_data: Record<string, never>;
    next_cue: Record<string, never>;
    previous_cue: Record<string, never>;
    jump_to_cue: { cue_number: number };
}
