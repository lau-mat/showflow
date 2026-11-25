// Mirrors protocol/src/sync.rs: ops are the one way every edit travels, from the browser or the
// app to the cloud, and changes are what the cloud hands back to everyone else.

export type EntityKind = "show" | "role" | "line" | "comment";
export type OpKind = "set" | "delete";

/** "Insert right after `after`" (null: at the start). The server turns it into a position key. */
export interface Place {
    after: string | null;
}

export interface Op {
    /** Idempotency key: the server applies an op id once, however often it is sent. */
    op_id: string;
    entity: EntityKind;
    id: string;
    kind: OpKind;
    fields?: Record<string, unknown>;
    place?: Place;
}

/** An applied op as the server recorded it (a `place` resolved into a concrete `position`). */
export interface Change {
    rev: number;
    op_id: string;
    entity: EntityKind;
    id: string;
    kind: OpKind;
    fields: Record<string, unknown>;
    author_id: string | null;
}

export type OpResult =
    | { status: "applied"; op_id: string; rev: number; position?: string }
    | { status: "duplicate"; op_id: string; rev: number }
    | { status: "rejected"; op_id: string; error: string };

export interface PushResponse {
    results: OpResult[];
    rev: number;
}

export interface PullResponse {
    changes: Change[];
    rev: number;
    more: boolean;
    /** The show was deleted: drop the local copy. */
    deleted: boolean;
}

export function newOp(entity: EntityKind, id: string, kind: OpKind, fields?: Record<string, unknown>, place?: Place): Op {
    const op: Op = { op_id: crypto.randomUUID(), entity, id, kind };
    if (fields) op.fields = fields;
    if (place) op.place = place;
    return op;
}
