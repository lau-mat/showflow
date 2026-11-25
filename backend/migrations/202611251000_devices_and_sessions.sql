-- Devices: the (previously unused) `client` table becomes the per-user device registry.
ALTER TABLE client
    ADD COLUMN client_app_version VARCHAR(32),
    ADD CONSTRAINT client_platform_known CHECK (client_platform IN ('web', 'desktop', 'mobile')),
    ADD CONSTRAINT client_identifier_per_user UNIQUE (user_id, client_identifier);

-- Sessions: sliding idle expiry bounded by an absolute cap, device link, and the previous
-- refresh token so a lost refresh response can be retried (and replays detected).
ALTER TABLE auth_sessions
    ADD COLUMN client_id UUID REFERENCES client(client_id) ON DELETE CASCADE,
    ADD COLUMN last_used_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ADD COLUMN absolute_expires_at TIMESTAMPTZ,
    ADD COLUMN previous_refresh_token_hash CHAR(64),
    ADD COLUMN rotated_at TIMESTAMPTZ;

UPDATE auth_sessions SET absolute_expires_at = expires_at;
ALTER TABLE auth_sessions ALTER COLUMN absolute_expires_at SET NOT NULL;

CREATE INDEX auth_sessions_client_id ON auth_sessions (client_id);
CREATE INDEX auth_sessions_previous_hash ON auth_sessions (previous_refresh_token_hash)
    WHERE previous_refresh_token_hash IS NOT NULL;

-- Single-use, hashed tokens for email verification and password reset. Nothing issues them
-- yet; this is the storage the future endpoints will use.
CREATE TABLE user_tokens (
    token_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    purpose VARCHAR(32) NOT NULL CHECK (purpose IN ('email_verify', 'password_reset')),
    token_hash CHAR(64) UNIQUE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ,
    CONSTRAINT user_token_expiry CHECK (expires_at > created_at)
);

CREATE INDEX user_tokens_user_id ON user_tokens (user_id);
CREATE INDEX user_tokens_expires_at ON user_tokens (expires_at);
