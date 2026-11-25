import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const { AuthSession } = await import("../frontend/main/js/auth/session.ts");
const { ApiError, NetworkError } = await import("../frontend/main/js/api/types.ts");
const { RestClient, createRestAuthApi } = await import("../frontend/main/js/api/rest.ts");

const tokens = (n, expiresIn = 900) => ({
    access_token: `access-${n}`, token_type: "Bearer", expires_in: expiresIn,
    refresh_token: `refresh-${n}`, refresh_expires_at: "2027-01-01T00:00:00Z",
    user: { id: "u1", username: "alice", email: "alice@example.com", email_verified: false }
});
// What the server sends to browsers: the refresh token travels as an HttpOnly cookie instead.
const cookieTokens = n => { const { refresh_token, ...rest } = tokens(n); return rest; };

function setup({ stored = {}, offlineGrace = true, cookieMode = false, api = {} } = {}) {
    let clock = 1_000_000;
    const log = [];
    const state = {
        refreshToken: null, refreshExpiresAt: null, deviceId: "dev", graceRemainingSeconds: 100, ...stored
    };
    const store = {
        load: async () => ({ hasSession: state.refreshToken !== null, ...state }),
        save: async (token, expires) => { log.push(["save", token]); state.refreshToken = token; state.refreshExpiresAt = expires; state.hasSession = true; },
        clear: async () => { log.push(["clear"]); state.refreshToken = null; state.hasSession = false; }
    };
    const fullApi = {
        login: async () => tokens(1),
        refresh: async () => tokens(2),
        logout: async token => { log.push(["logout", token]); },
        me: async () => ({ user_id: "u" }),
        register: async () => tokens(1),
        logoutAll: async () => { log.push(["logoutAll"]); },
        ...api
    };
    const session = new AuthSession({ api: fullApi, store, offlineGrace, cookieMode, now: () => clock });
    return { session, log, state, advance: ms => { clock += ms; } };
}

test("without a stored token the user must log in", async () => {
    const { session } = setup();
    assert.equal(await session.restore(), "unauthenticated");
    assert.equal(session.isAuthorized, false);
});

test("login stores the refresh token and authorizes the session", async () => {
    const { session, log, state } = setup();
    await session.login("a@b.co", "pw");
    assert.equal(session.state, "authenticated");
    assert.equal(state.refreshToken, "refresh-1");
    assert.deepEqual(log, [["save", "refresh-1"]]);
    assert.equal(await session.getAccessToken(), "access-1");
});

test("a valid stored token is refreshed online and the rotated token is persisted", async () => {
    const { session, state } = setup({ stored: { refreshToken: "refresh-0" } });
    assert.equal(await session.restore(), "authenticated");
    assert.equal(state.refreshToken, "refresh-2");
});

test("a rejected stored token (401) is cleared and sends the user to login", async () => {
    const { session, log } = setup({
        stored: { refreshToken: "old" },
        api: { refresh: async () => { throw new ApiError(401, "Invalid credentials or token"); } }
    });
    assert.equal(await session.restore(), "unauthenticated");
    assert.deepEqual(log, [["clear"]]);
});

test("apps keep working offline inside the grace window and lock once it is over", async () => {
    const offline = { refresh: async () => { throw new NetworkError(); } };
    const inside = setup({ stored: { refreshToken: "t", graceRemainingSeconds: 60 }, api: offline });
    assert.equal(await inside.session.restore(), "offlineGrace");
    assert.equal(inside.session.isAuthorized, true);

    const outside = setup({ stored: { refreshToken: "t", graceRemainingSeconds: 0 }, api: offline });
    assert.equal(await outside.session.restore(), "graceExpired");
    assert.equal(outside.session.isAuthorized, false);
    assert.equal(outside.log.some(entry => entry[0] === "clear"), false, "token kept so a later online restore can work");
});

test("a 5xx from the cloud counts as unreachable, not as a rejected login", async () => {
    const { session } = setup({
        stored: { refreshToken: "t", graceRemainingSeconds: 60 },
        api: { refresh: async () => { throw new ApiError(503, "unavailable"); } }
    });
    assert.equal(await session.restore(), "offlineGrace");
});

test("the web has no offline grace: an unreachable cloud means unauthenticated", async () => {
    const { session } = setup({
        offlineGrace: false,
        stored: { refreshToken: "t", graceRemainingSeconds: null },
        api: { refresh: async () => { throw new NetworkError(); } }
    });
    assert.equal(await session.restore(), "unauthenticated");
});

test("coming back online re-validates a session that was in offline grace", async () => {
    let online = false;
    const { session, state } = setup({
        stored: { refreshToken: "t", graceRemainingSeconds: 60 },
        api: { refresh: async () => { if (!online) throw new NetworkError(); return tokens(5); } }
    });
    await session.restore();
    assert.equal(session.state, "offlineGrace");
    online = true;
    assert.equal(await session.retryOnline(), "authenticated");
    assert.equal(state.refreshToken, "refresh-5");
});

test("concurrent refreshes share one request because refresh tokens are single-use", async () => {
    let calls = 0;
    const { session } = setup({
        stored: { refreshToken: "t" },
        api: { refresh: async () => { calls++; await new Promise(r => setImmediate(r)); return tokens(calls + 10); } }
    });
    await session.restore();
    calls = 0;
    const [a, b, c] = await Promise.all([session.forceRefresh(), session.forceRefresh(), session.forceRefresh()]);
    assert.equal(calls, 1);
    assert.equal(a, b);
    assert.equal(b, c);
});

test("the rotated refresh token is persisted before it replaces the one in memory", async () => {
    const order = [];
    const { session } = setup({ stored: { refreshToken: "t" } });
    await session.restore();
    const originalSave = session.deps.store.save;
    session.deps.store.save = async (...args) => { order.push("save"); await originalSave(...args); order.push("saved"); };
    session.deps.api.refresh = async token => { order.push(`refresh:${token}`); return tokens(9); };
    await session.forceRefresh();
    assert.deepEqual(order, ["refresh:refresh-2", "save", "saved"]);
});

test("access tokens are reused until shortly before they expire", async () => {
    let refreshes = 0;
    const { session, advance } = setup({
        stored: { refreshToken: "t" },
        api: { refresh: async () => { refreshes++; return tokens(refreshes, 900); } }
    });
    await session.restore();
    assert.equal(refreshes, 1);
    advance(800_000);
    await session.getAccessToken();
    assert.equal(refreshes, 1);
    advance(80_000); // inside the 30s safety margin of the 900s lifetime
    await session.getAccessToken();
    assert.equal(refreshes, 2);
});

test("logout clears local state first and tells the server on a best-effort basis", async () => {
    const { session, log } = setup({
        api: { logout: async () => { throw new NetworkError(); } }
    });
    await session.login("a@b.co", "pw");
    log.length = 0;
    await session.logout();
    assert.equal(session.state, "unauthenticated");
    assert.deepEqual(log, [["clear"]]);
});

test("state listeners are told about every transition", async () => {
    const { session } = setup();
    const seen = [];
    const unsubscribe = session.subscribe(state => seen.push(state));
    await session.login("a@b.co", "pw");
    await session.logout();
    unsubscribe();
    await session.restore();
    assert.deepEqual(seen, ["authenticated", "unauthenticated"]);
});

test("REST client sends the client headers, retries a 401 once with a refreshed token, and maps failures", async () => {
    const requests = [];
    const responses = [
        { status: 401, ok: false, json: async () => ({ error: "expired" }) },
        { status: 200, ok: true, json: async () => ({ user_id: "u" }) }
    ];
    globalThis.fetch = async (url, init) => { requests.push({ url, init }); return responses.shift(); };
    let refreshed = 0;
    const client = new RestClient({
        baseUrl: "/api",
        clientHeaders: () => ({ "X-ShowFlow-Platform": "web/web", "X-ShowFlow-Device-Id": "dev" }),
        getAccessToken: async () => "stale",
        refreshAccessToken: async () => { refreshed++; return "fresh"; },
        credentials: "omit"
    });
    assert.deepEqual(await client.request("auth/me", { authorized: true }), { user_id: "u" });
    assert.equal(refreshed, 1);
    assert.equal(requests[0].init.headers.Authorization, "Bearer stale");
    assert.equal(requests[1].init.headers.Authorization, "Bearer fresh");
    assert.equal(requests[1].init.headers["X-ShowFlow-Device-Id"], "dev");
    assert.equal(requests[1].url, "/api/auth/me");

    globalThis.fetch = async () => ({ status: 401, ok: false, json: async () => ({ error: "Invalid credentials or token" }) });
    const api = createRestAuthApi(client);
    await assert.rejects(api.login("a@b.co", "x"), error => error instanceof ApiError && error.status === 401 && /Invalid credentials/.test(error.message));

    globalThis.fetch = async () => { throw new TypeError("Failed to fetch"); };
    await assert.rejects(api.login("a@b.co", "x"), NetworkError);
});

test("registering signs in with the new account and exposes it", async () => {
    let sent;
    const { session, state } = setup({ api: { register: async (...args) => { sent = args; return tokens(1); } } });
    await session.register("alice", "alice@example.com", "a long password");
    assert.deepEqual(sent, ["alice", "alice@example.com", "a long password"]);
    assert.equal(session.state, "authenticated");
    assert.equal(session.user.username, "alice");
    assert.equal(state.refreshToken, "refresh-1");
});

test("a registration the server rejects leaves the user signed out", async () => {
    const { session } = setup({ api: { register: async () => { throw new ApiError(409, "Username or email is already registered"); } } });
    await assert.rejects(session.register("a", "a@b.co", "x"), error => error.status === 409);
    assert.equal(session.state, "unauthenticated");
    assert.equal(session.user, null);
});

test("cookie mode: JS never holds or stores a refresh token", async () => {
    const calls = [];
    const { session, log, state } = setup({
        cookieMode: true, offlineGrace: false,
        api: { login: async () => cookieTokens(1), refresh: async token => { calls.push(token); return cookieTokens(2); } }
    });
    await session.login("a@b.co", "pw");
    assert.deepEqual(log, [["save", null]]);
    assert.equal(state.refreshToken, null);
    assert.equal(await session.getAccessToken(), "access-1");
    assert.equal(await session.forceRefresh(), "access-2");
    assert.deepEqual(calls, [null], "the cookie is sent by the browser, not by us");
});

test("cookie mode: restore only asks the server when a login is likely", async () => {
    const calls = [];
    const api = { refresh: async token => { calls.push(token); return cookieTokens(2); } };
    const cold = setup({ cookieMode: true, offlineGrace: false, api });
    assert.equal(await cold.session.restore(), "unauthenticated");
    assert.deepEqual(calls, [], "no hint, so no request (and no 401 in the console)");

    const warm = setup({ cookieMode: true, offlineGrace: false, api, stored: { hasSession: true } });
    assert.equal(await warm.session.restore(), "authenticated");
    assert.deepEqual(calls, [null]);
    assert.equal(warm.session.user.email, "alice@example.com");

    const expired = setup({
        cookieMode: true, offlineGrace: false, stored: { hasSession: true },
        api: { refresh: async () => { throw new ApiError(401, "Invalid credentials or token"); } }
    });
    assert.equal(await expired.session.restore(), "unauthenticated");
    assert.ok(expired.log.some(entry => entry[0] === "clear"), "the stale hint is dropped");
});

test("cookie mode: an unreachable server on the web is logged out, not offline-granted", async () => {
    const { session } = setup({
        cookieMode: true, offlineGrace: false, stored: { hasSession: true },
        api: { refresh: async () => { throw new NetworkError(); } }
    });
    assert.equal(await session.restore(), "unauthenticated");
});

test("cookie mode: logout is only complete once the server has cleared the cookie", async () => {
    const { session, log } = setup({
        cookieMode: true, offlineGrace: false,
        api: { login: async () => cookieTokens(1), logout: async token => { log.push(["logout", token]); } }
    });
    await session.login("a@b.co", "pw");
    await session.logout();
    assert.deepEqual(log.slice(-2), [["logout", null], ["clear"]]);
    assert.equal(session.state, "unauthenticated");
    assert.equal(session.user, null);

    const failing = setup({
        cookieMode: true, offlineGrace: false,
        api: { login: async () => cookieTokens(1), logout: async () => { throw new NetworkError(); } }
    });
    await failing.session.login("a@b.co", "pw");
    await assert.rejects(failing.session.logout(), NetworkError);
    assert.equal(failing.session.state, "authenticated", "still signed in when the cookie could not be cleared");
});

test("apps: logout forgets locally even when the server cannot be reached", async () => {
    const { session, log } = setup({ api: { logout: async () => { throw new NetworkError(); } } });
    await session.login("a@b.co", "pw");
    await session.logout();
    assert.equal(session.state, "unauthenticated");
    assert.ok(log.some(entry => entry[0] === "clear"));
});

test("logging out everywhere ends this session as well", async () => {
    const { session, log } = setup();
    await session.login("a@b.co", "pw");
    await session.logoutEverywhere();
    assert.equal(session.state, "unauthenticated");
    assert.ok(log.some(entry => entry[0] === "logoutAll"));
    await assert.rejects(session.forceRefresh(), ApiError);
});

test("the REST auth api: cookie requests carry no body and use the right credentials/verbs", async () => {
    const requests = [];
    globalThis.fetch = async (url, init) => { requests.push({ url, init }); return { status: 204, ok: true }; };
    const client = new RestClient({
        baseUrl: "/api", clientHeaders: () => ({ "X-ShowFlow-Platform": "web/web" }),
        getAccessToken: async () => "tok", credentials: "same-origin"
    });
    const api = createRestAuthApi(client);

    await api.refresh(null);
    await api.logout(null);
    await api.refresh("refresh-9");
    await api.revokeDevice("dev-1");
    await api.logoutAll();
    await api.register("alice", "a@b.co", "pw");

    const [refreshCookie, logoutCookie, refreshApp, revoke, all, register] = requests;
    assert.equal(refreshCookie.init.method, "POST");
    assert.equal(refreshCookie.init.body, undefined);
    assert.equal(refreshCookie.init.credentials, "same-origin");
    assert.equal(logoutCookie.url, "/api/auth/logout");
    assert.equal(refreshApp.init.body, JSON.stringify({ refresh_token: "refresh-9" }));
    assert.equal(revoke.url, "/api/auth/devices/dev-1");
    assert.equal(revoke.init.method, "DELETE");
    assert.equal(revoke.init.headers.Authorization, "Bearer tok");
    assert.equal(all.url, "/api/auth/logout-all");
    assert.equal(register.url, "/api/auth/register");
    assert.deepEqual(JSON.parse(register.init.body), { username: "alice", email: "a@b.co", password: "pw" });
});
