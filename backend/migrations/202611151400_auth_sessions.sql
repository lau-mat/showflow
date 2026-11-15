-- Refuse ambiguous existing accounts rather than allowing case-variant logins.
CREATE UNIQUE INDEX users_email_case_insensitive ON users (lower(user_email));

CREATE TABLE auth_sessions (
    session_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    refresh_token_hash CHAR(64) UNIQUE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT auth_session_expiry CHECK (expires_at > created_at)
);

CREATE INDEX auth_sessions_user_id ON auth_sessions (user_id);
CREATE INDEX auth_sessions_expires_at ON auth_sessions (expires_at);
