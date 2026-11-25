-- Replaces the placeholder show tables from the initial schema (nothing ever wrote to them)
-- with the shape the sync protocol needs: client-generated UUID ids, per-show revision
-- numbers, a change log, soft deletes, and members.

DROP TABLE IF EXISTS consumption;
DROP TABLE IF EXISTS show_line_comment;
DROP TABLE IF EXISTS show_line;
DROP TABLE IF EXISTS show_role;
DROP TABLE IF EXISTS shows;
DROP TABLE IF EXISTS "group";

CREATE TABLE shows (
    show_id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    show_time BIGINT NOT NULL,
    -- Number of the last change applied; assigned under a row lock, so it never has gaps.
    rev BIGINT NOT NULL DEFAULT 0,
    created_by UUID REFERENCES users(user_id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMPTZ
);

CREATE TABLE show_roles (
    role_id UUID PRIMARY KEY,
    show_id UUID NOT NULL REFERENCES shows(show_id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX show_roles_show ON show_roles (show_id);

CREATE TABLE show_lines (
    line_id UUID PRIMARY KEY,
    show_id UUID NOT NULL REFERENCES shows(show_id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    duration BIGINT NOT NULL DEFAULT 0,
    absolute_time BIGINT,
    -- Fractional ordering key. Byte order is the order, hence the "C" collation: a locale
    -- collation would sort 'a' next to 'A' and scramble the script.
    position TEXT COLLATE "C" NOT NULL,
    updated_by UUID REFERENCES users(user_id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX show_lines_order ON show_lines (show_id, position, line_id);

CREATE TABLE show_comments (
    comment_id UUID PRIMARY KEY,
    show_id UUID NOT NULL REFERENCES shows(show_id) ON DELETE CASCADE,
    line_id UUID NOT NULL REFERENCES show_lines(line_id) ON DELETE CASCADE,
    role_id UUID REFERENCES show_roles(role_id) ON DELETE SET NULL,
    comment TEXT NOT NULL,
    deleted_at TIMESTAMPTZ
);
CREATE INDEX show_comments_show ON show_comments (show_id);

-- The change log: what clients pull (`rev > since`) and the record that makes retries safe.
CREATE TABLE show_changes (
    show_id UUID NOT NULL REFERENCES shows(show_id) ON DELETE CASCADE,
    rev BIGINT NOT NULL,
    op_id UUID NOT NULL,
    entity VARCHAR(16) NOT NULL CHECK (entity IN ('show', 'role', 'line', 'comment')),
    entity_id UUID NOT NULL,
    kind VARCHAR(8) NOT NULL CHECK (kind IN ('set', 'delete')),
    fields JSONB NOT NULL DEFAULT '{}'::jsonb,
    author_id UUID REFERENCES users(user_id) ON DELETE SET NULL,
    client_id UUID REFERENCES client(client_id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (show_id, rev),
    UNIQUE (show_id, op_id)
);

-- Sharing. One row per user and show; teams can later add a `team_id` next to `user_id`.
CREATE TABLE show_members (
    show_id UUID NOT NULL REFERENCES shows(show_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    role VARCHAR(16) NOT NULL CHECK (role IN ('owner', 'editor', 'commenter', 'viewer')),
    show_role_id UUID REFERENCES show_roles(role_id) ON DELETE SET NULL,
    added_by UUID REFERENCES users(user_id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (show_id, user_id)
);
CREATE INDEX show_members_user ON show_members (user_id);

CREATE TABLE show_invites (
    invite_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    show_id UUID NOT NULL REFERENCES shows(show_id) ON DELETE CASCADE,
    role VARCHAR(16) NOT NULL CHECK (role IN ('editor', 'commenter', 'viewer')),
    token_hash CHAR(64) UNIQUE NOT NULL,
    -- When set, only the account with this email may accept.
    email VARCHAR(255),
    created_by UUID REFERENCES users(user_id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ NOT NULL,
    max_uses INTEGER NOT NULL DEFAULT 1 CHECK (max_uses > 0),
    used_count INTEGER NOT NULL DEFAULT 0,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX show_invites_show ON show_invites (show_id);

CREATE TABLE consumption (
    consumption_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    show_id UUID REFERENCES shows(show_id) ON DELETE CASCADE,
    consumption_date TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
);
