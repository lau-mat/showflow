import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const { buildProfile } = await import("../frontend/shared/js/platform.ts");
const { Router, resolvePage } = await import("../frontend/main/js/page_loader.ts");

const noop = () => {};
const pages = {
    login: { load: noop, access: "public" },
    register: { load: noop, access: "public" },
    home: { load: noop, access: "auth", feature: "showEditor" },
    sessions: { load: noop, access: "auth", feature: "sessionDiscovery" }
};
const web = buildProfile("web", "web");
const desktop = buildProfile("desktop", "linux");
const mobile = buildProfile("mobile", "android");

test("unauthenticated users are sent to the platform login page", () => {
    for (const profile of [web, desktop, mobile]) {
        assert.equal(resolvePage("home", pages, profile, false), "login");
        assert.equal(resolvePage("sessions", pages, profile, false), "login");
    }
    assert.equal(resolvePage("login", pages, web, false), "login");
});

test("authorized users reach pages; unknown pages fall back to the platform landing page", () => {
    assert.equal(resolvePage("home", pages, desktop, true), "home");
    assert.equal(resolvePage("nope", pages, desktop, true), "home");
    assert.equal(resolvePage("nope", pages, mobile, true), "sessions");
    assert.equal(resolvePage("nope", pages, web, false), "login");
});

test("pages that need a missing feature fall back to the landing page", () => {
    assert.equal(resolvePage("sessions", pages, web, true), "home");
    assert.equal(resolvePage("sessions", pages, mobile, true), "sessions");
});

test("signed-out users on every platform can reach the register page", () => {
    for (const profile of [web, desktop, mobile]) {
        assert.equal(resolvePage(profile.registerPage, pages, profile, false), "register");
    }
});

function setupRouter(profile, authorized) {
    const listeners = {};
    const loaded = [];
    const emptied = [];
    const win = {
        location: { hash: "" },
        addEventListener: (type, listener) => { listeners[type] = listener; }
    };
    const doc = {
        title: "",
        getElementById: () => ({ replaceChildren: () => emptied.push(true) })
    };
    const state = { authorized };
    const registry = Object.fromEntries(Object.entries(pages).map(([name, page]) => [
        name, { ...page, load: () => { loaded.push(name); } }
    ]));
    const router = new Router({ pages: registry, profile, isAuthorized: () => state.authorized, win, doc });
    // A browser fires hashchange after the hash is assigned.
    let hash = "";
    Object.defineProperty(win.location, "hash", {
        get: () => hash,
        set: value => { hash = value; queueMicrotask(() => listeners.hashchange?.()); }
    });
    return { router, win, doc, loaded, emptied, state, setHash: value => { hash = value; } };
}

const settle = () => new Promise(resolve => setImmediate(resolve));

test("starting without a hash lands unauthenticated users on login and authorized ones on the landing page", async () => {
    const anon = setupRouter(desktop, false);
    await anon.router.start();
    await settle();
    assert.equal(anon.win.location.hash, "#login");
    assert.deepEqual(anon.loaded, ["login"]);
    assert.equal(anon.doc.title, "Showflow - Login");

    const user = setupRouter(mobile, true);
    await user.router.start();
    await settle();
    assert.equal(user.win.location.hash, "#sessions");
    assert.deepEqual(user.loaded, ["sessions"]);
});

test("each page renders once per navigation and the main section is cleared first", async () => {
    const app = setupRouter(desktop, true);
    app.setHash("home");
    await app.router.start();
    await settle();
    assert.deepEqual(app.loaded, ["home"]);
    assert.equal(app.emptied.length, 1);
});

test("guards are re-checked on every navigation, including hash edits", async () => {
    const app = setupRouter(web, true);
    app.setHash("home");
    await app.router.start();
    await settle();
    app.state.authorized = false; // session lost (e.g. logout)
    app.win.location.hash = "#home";
    await settle();
    assert.equal(app.win.location.hash, "#login");
    assert.deepEqual(app.loaded, ["home", "login"]);
});

test("pages receive what follows their name in the hash", async () => {
    const { win, loaded, router } = (() => {
        const listeners = {};
        const loaded = [];
        const win = { location: { hash: "" }, addEventListener: (event, cb) => { listeners[event] = cb; } };
        const doc = { title: "", getElementById: () => ({ replaceChildren() {} }) };
        const router = new Router({
            pages: { ...pages, invite: { load: params => { loaded.push(["invite", params]); }, access: "auth" } },
            profile: web, isAuthorized: () => true, win, doc
        });
        return { win, loaded, router };
    })();
    win.location.hash = "#invite/abc/def";
    await router.navigate("invite/abc/def");
    assert.deepEqual(loaded, [["invite", "abc/def"]]);
    assert.equal(win.location.hash, "#invite/abc/def", "the token stays in the address");
});

test("a signed-out visitor to a protected page is sent to login and remembered", async () => {
    const store = new Map();
    globalThis.sessionStorage = { getItem: key => store.get(key) ?? null, setItem: (key, value) => store.set(key, value), removeItem: key => store.delete(key) };
    const { takeReturnTo } = await import("../frontend/main/js/page_loader.ts");
    const win = { location: { hash: "#invite/token123" }, addEventListener() {} };
    const doc = { title: "", getElementById: () => ({ replaceChildren() {} }) };
    const router = new Router({
        pages: { ...pages, invite: { load: () => {}, access: "auth" } },
        profile: web, isAuthorized: () => false, win, doc
    });
    await router.navigate("invite/token123");
    assert.equal(win.location.hash, "#login");
    assert.equal(takeReturnTo(), "invite/token123");
    assert.equal(takeReturnTo(), null, "it is only used once");
    delete globalThis.sessionStorage;
});

test("returning to a page after signing in works without any storage too", async () => {
    const { takeReturnTo } = await import("../frontend/main/js/page_loader.ts");
    assert.equal(takeReturnTo(), null);
});
