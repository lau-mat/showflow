import assert from "node:assert/strict";
import test from "node:test";
import "./helpers/ts_hooks.js";

const platform = await import("../frontend/shared/js/platform.ts");

function reset(tauri, nativeInfo) {
    platform.installPlatform(null);
    globalThis.window = tauri ? { __TAURI_INTERNALS__: {} } : {};
    globalThis.__invoke = async command => {
        assert.equal(command, "platform_info");
        return nativeInfo;
    };
}

test("without the Tauri runtime the platform is the webapp, using REST", async () => {
    reset(false);
    const profile = await platform.detectPlatform();
    assert.equal(profile.kind, "web");
    assert.equal(profile.transport, "rest");
    assert.equal(profile.offlineGraceDays, null);
    assert.equal(profile.landingPage, "home");
});

test("inside Tauri the native platform command decides between desktop and mobile", async () => {
    reset(true, { kind: "desktop", os: "linux" });
    const desktop = await platform.detectPlatform();
    assert.deepEqual([desktop.kind, desktop.os, desktop.transport], ["desktop", "linux", "ipc"]);

    reset(true, { kind: "mobile", os: "android" });
    const mobile = await platform.detectPlatform();
    assert.deepEqual([mobile.kind, mobile.os, mobile.transport], ["mobile", "android", "ipc"]);
    assert.equal(mobile.landingPage, "sessions");
});

test("detection runs once and is memoised", async () => {
    reset(true, { kind: "desktop", os: "windows" });
    let calls = 0;
    globalThis.__invoke = async () => { calls++; return { kind: "desktop", os: "windows" }; };
    const [a, b] = await Promise.all([platform.detectPlatform(), platform.detectPlatform()]);
    assert.equal(a, b);
    assert.equal(calls, 1);
    assert.equal(platform.getPlatform(), a);
});

test("a failed detection is not cached and getPlatform throws until detected", async () => {
    reset(true);
    assert.throws(() => platform.getPlatform(), /not detected yet/);
    globalThis.__invoke = async () => { throw new Error("ipc down"); };
    await assert.rejects(platform.detectPlatform(), /ipc down/);
    globalThis.__invoke = async () => ({ kind: "mobile", os: "ios" });
    assert.equal((await platform.detectPlatform()).os, "ios");
});

test("feature table: apps keep local data, only desktop hosts sessions, only web lacks offline use", () => {
    const desktop = platform.buildProfile("desktop", "linux").features;
    const mobile = platform.buildProfile("mobile", "ios").features;
    const web = platform.buildProfile("web", "web").features;

    for (const features of [desktop, mobile, web]) {
        assert.equal(features.showEditor, true);
        assert.equal(features.sessionView, true);
    }
    assert.deepEqual([desktop.sessionHost, mobile.sessionHost, web.sessionHost], [true, false, false]);
    assert.deepEqual([desktop.qrShare, mobile.qrShare, web.qrShare], [true, false, false]);
    assert.deepEqual([desktop.fullscreenWindow, mobile.fullscreenWindow, web.fullscreenWindow], [true, false, false]);
    assert.deepEqual([desktop.sessionDiscovery, mobile.sessionDiscovery, web.sessionDiscovery], [true, true, false]);
    assert.deepEqual([desktop.print, mobile.print, web.print], [true, false, true]);
    assert.deepEqual([desktop.offlineData, mobile.offlineData, web.offlineData], [true, true, false]);
});

test("hasFeature reads the detected profile", () => {
    platform.installPlatform(platform.buildProfile("mobile", "android"));
    assert.equal(platform.hasFeature("sessionHost"), false);
    assert.equal(platform.hasFeature("showEditor"), true);
});

test("token storage and registration are table-driven per platform", async () => {
    const { PROFILES } = await import("../frontend/shared/js/platform.ts");
    assert.equal(PROFILES.web.tokenStorage, "cookie");
    assert.equal(PROFILES.desktop.tokenStorage, "device");
    assert.equal(PROFILES.mobile.tokenStorage, "device");
    for (const kind of ["web", "desktop", "mobile"]) assert.equal(PROFILES[kind].registerPage, "register");
});
