import { ApiError, NetworkError, type AuthApi, type AuthUser, type TokenResponse } from "../api/types";
import type { TokenStore } from "./token_store";

/**
 * unauthenticated: must log in.
 * authenticated:   logged in and validated online.
 * offlineGrace:    apps only; cloud unreachable but the last online validation is recent enough.
 * graceExpired:    apps only; offline for longer than the grace window, must go online and log in.
 */
export type AuthState = "unauthenticated" | "authenticated" | "offlineGrace" | "graceExpired";

export interface AuthSessionDeps {
    api: AuthApi;
    store: TokenStore;
    /** Apps have an offline grace window; the web does not. */
    offlineGrace: boolean;
    /**
     * Browsers: the refresh token is an HttpOnly cookie. JS never holds it; requests that need
     * it simply send no token and the browser attaches the cookie.
     */
    cookieMode?: boolean;
    now?: () => number;
}

/** Refresh slightly early so a request never leaves with an about-to-expire token. */
const EXPIRY_MARGIN_MS = 30_000;

export class AuthSession {
    state: AuthState = "unauthenticated";
    /** The signed-in account (kept from the last login/refresh; null when signed out). */
    user: AuthUser | null = null;
    private loggedIn = false;
    private accessToken: string | null = null;
    private accessExpiresAt = 0;
    private refreshToken: string | null = null;
    private inflight: Promise<TokenResponse> | null = null;
    private listeners = new Set<(state: AuthState) => void>();
    private readonly deps: AuthSessionDeps;
    private readonly now: () => number;

    constructor(deps: AuthSessionDeps) {
        this.deps = deps;
        this.now = deps.now ?? Date.now;
    }

    /** True when the user may use the app (online, or inside the offline grace window). */
    get isAuthorized(): boolean {
        return this.state === "authenticated" || this.state === "offlineGrace";
    }

    subscribe(listener: (state: AuthState) => void): () => void {
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
    }

    /** Resumes a stored session at startup, or when connectivity returns. */
    async restore(): Promise<AuthState> {
        const stored = await this.deps.store.load();
        if (!stored.hasSession) return this.setState("unauthenticated");

        try {
            await this.refreshWith(stored.refreshToken);
            return this.setState("authenticated");
        } catch (error) {
            if (error instanceof ApiError && error.isRejection) {
                await this.forget();
                return this.setState("unauthenticated");
            }
            // Cloud unreachable (network error or 5xx): only apps may carry on offline.
            if (!this.deps.offlineGrace) return this.setState("unauthenticated");
            this.refreshToken = stored.refreshToken;
            this.loggedIn = true;
            return this.setState((stored.graceRemainingSeconds ?? 0) > 0 ? "offlineGrace" : "graceExpired");
        }
    }

    /** Re-validates online while running inside the offline grace window. */
    async retryOnline(): Promise<AuthState> {
        if (this.state !== "offlineGrace") return this.state;
        return this.restore();
    }

    async login(email: string, password: string): Promise<void> {
        await this.start(await this.deps.api.login(email, password));
    }

    /** Creates the account and signs in with it in one step. */
    async register(username: string, email: string, password: string): Promise<void> {
        await this.start(await this.deps.api.register(username, email, password));
    }

    async logout(): Promise<void> {
        if (this.deps.cookieMode) {
            // The server has to clear the HttpOnly cookie; staying signed in beats silently
            // leaving a live cookie behind, so a failure is reported instead of swallowed.
            await this.deps.api.logout(null);
            await this.forget();
            this.setState("unauthenticated");
            return;
        }
        const token = this.refreshToken;
        await this.forget();
        this.setState("unauthenticated");
        // Best effort: the local session is gone either way.
        if (token) await this.deps.api.logout(token).catch(() => undefined);
    }

    /** Ends every login of this account (all devices), including this one. */
    async logoutEverywhere(): Promise<void> {
        await this.deps.api.logoutAll();
        await this.forget();
        this.setState("unauthenticated");
    }

    private async start(tokens: TokenResponse): Promise<void> {
        await this.adopt(tokens);
        this.setState("authenticated");
    }

    /** A valid access token, refreshing first when it is missing or about to expire. */
    async getAccessToken(): Promise<string> {
        if (this.accessToken && this.now() < this.accessExpiresAt - EXPIRY_MARGIN_MS) return this.accessToken;
        return this.forceRefresh();
    }

    /** Refreshes regardless of expiry; used after a 401. */
    async forceRefresh(): Promise<string> {
        if (!this.loggedIn) throw new ApiError(401, "Not logged in");
        return (await this.refreshWith(this.refreshToken)).access_token;
    }

    /** Concurrent callers share one request: refresh tokens are single-use on the server. */
    private refreshWith(refreshToken: string | null): Promise<TokenResponse> {
        this.inflight ??= this.deps.api.refresh(refreshToken)
            .then(async tokens => {
                await this.adopt(tokens);
                return tokens;
            })
            .finally(() => { this.inflight = null; });
        return this.inflight;
    }

    /** Persists the rotated refresh token before the old one is dropped from memory. */
    private async adopt(tokens: TokenResponse): Promise<void> {
        // Cookie mode returns no refresh token; the store then only records that a login exists.
        const refreshToken = this.deps.cookieMode ? null : tokens.refresh_token ?? null;
        if (!this.deps.cookieMode && refreshToken === null) throw new ApiError(500, "The server returned no refresh token.");
        await this.deps.store.save(refreshToken, tokens.refresh_expires_at, tokens.user?.id ?? null);
        this.refreshToken = refreshToken;
        this.loggedIn = true;
        this.user = tokens.user ?? this.user;
        this.accessToken = tokens.access_token;
        this.accessExpiresAt = this.now() + tokens.expires_in * 1000;
    }

    private async forget(): Promise<void> {
        this.loggedIn = false;
        this.user = null;
        this.refreshToken = null;
        this.accessToken = null;
        this.accessExpiresAt = 0;
        await this.deps.store.clear();
    }

    private setState(state: AuthState): AuthState {
        this.state = state;
        this.listeners.forEach(listener => listener(state));
        return state;
    }
}

export { ApiError, NetworkError };
