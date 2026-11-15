-- Create Users Table
CREATE TABLE IF NOT EXISTS users (
    user_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_name VARCHAR(255) UNIQUE NOT NULL,
    user_email VARCHAR(255) UNIQUE NOT NULL,
    user_pass VARCHAR(255) NOT NULL,
    user_created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    user_last_login TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    user_active BOOLEAN DEFAULT TRUE,
    user_email_verified BOOLEAN DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS verification (
    verification_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    verification_token VARCHAR(255) NOT NULL,
    verification_created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    verification_expires_at TIMESTAMP WITH TIME ZONE NOT NULL
);

CREATE TABLE IF NOT EXISTS subscription (
    subscription_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    subscription_plan VARCHAR(255) NOT NULL,
    subscription_start TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    subscription_end TIMESTAMP WITH TIME ZONE
);

CREATE TABLE IF NOT EXISTS client (
    client_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    client_platform VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    client_identifier VARCHAR(255) NOT NULL,
    client_last_connected TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS "group" (
    group_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    group_name VARCHAR(255) NOT NULL,
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS shows (
    show_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    group_id UUID REFERENCES "group"(group_id) ON DELETE CASCADE,
    show_name VARCHAR(255) NOT NULL,
    show_start TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    show_running BOOLEAN DEFAULT FALSE,
    show_pass VARCHAR(255) NOT NULL
);

CREATE TABLE IF NOT EXISTS consumption (
    consumption_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    show_id UUID REFERENCES shows(show_id) ON DELETE CASCADE,
    consumption_date TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS show_line (
    show_line_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    show_id UUID REFERENCES shows(show_id) ON DELETE CASCADE,
    show_line_note TEXT NOT NULL,
    show_line_duration INTEGER NOT NULL,
    show_line_start_time TIMESTAMP WITH TIME ZONE NOT NULL
);

CREATE TABLE IF NOT EXISTS show_role (
    show_role_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    show_id UUID REFERENCES shows(show_id) ON DELETE CASCADE,
    role_name VARCHAR(255) NOT NULL
);

CREATE TABLE IF NOT EXISTS show_line_comment (
    show_line_comment_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    show_line_id UUID REFERENCES show_line(show_line_id) ON DELETE CASCADE,
    show_line_comment TEXT NOT NULL,
    show_role_id UUID REFERENCES show_role(show_role_id) ON DELETE CASCADE
);