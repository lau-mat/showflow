// Mirrors the serialized structs in app/src/lib.rs (get_full_show_details).

export interface Show {
    id: number;
    name: string;
    time: number;
}

export interface ShowRole {
    id: number;
    show_id: number;
    name: string;
}

export interface ScenarioLine {
    id: number;
    order: number;
    name: string;
    comment: string;
    duration: number;
    absolute_time: number | null;
}

export interface ScenarioLineComment {
    id: number;
    line_id: number;
    role_id: number | null;
    comment: string;
}

export interface ShowDetails {
    show: Show;
    roles: ShowRole[];
    lines: ScenarioLine[];
    comments: ScenarioLineComment[];
}
