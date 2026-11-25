// Emulated-device check of the layouts: node tests/e2e/responsive.check.mjs <dev server url> <screenshot dir>
//
// Needs Chrome (google-chrome-stable) and a running Vite dev server (e.g. `npx vite --config
// vite.config.js --port 1421`); no backend. The app runs against a faked Tauri runtime, so it
// behaves like the mobile/desktop apps. For every screen on every device it asserts: nothing
// overflows horizontally, dialogs fit the screen, and on touch devices all controls are at least
// 44px with 16px form text. Screenshots are written for a visual review.
// Deliberately not named *.test.js: `node --test tests/` stays free of a browser.

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [BASE = "http://127.0.0.1:1421", OUT = "."] = process.argv.slice(2);
mkdirSync(OUT, { recursive: true });
const sleep = ms => new Promise(r => setTimeout(r, ms));

// Breakpoint: compact layout up to 880px wide (phones and tablets in portrait).
const DEVICES = [
    { name: "phone-360", width: 360, height: 740, touch: true, profile: "mobile" },
    { name: "phone-390", width: 390, height: 844, touch: true, profile: "mobile" },
    { name: "phone-landscape", width: 844, height: 390, touch: true, profile: "mobile" },
    { name: "tablet-768", width: 768, height: 1024, touch: true, profile: "mobile" },
    { name: "tablet-landscape", width: 1024, height: 768, touch: true, profile: "mobile" },
    { name: "desktop-1280", width: 1280, height: 800, touch: false, profile: "desktop" }
];

const chrome = spawn("google-chrome-stable", ["--headless=new", "--no-sandbox", "--disable-gpu", "--remote-debugging-port=9343",
    `--user-data-dir=${mkdtempSync(join(tmpdir(), "resp-"))}`, "about:blank"], { stdio: "ignore" });
let version;
for (let i = 0; i < 50 && !version; i++) { try { version = await (await fetch("http://127.0.0.1:9343/json/version")).json(); } catch { await sleep(200); } }
if (!version) throw new Error("Chrome did not start");

let failures = 0;
const check = (name, ok, extra = "") => { console.log(`${ok ? "PASS" : "FAIL"}  ${name}${extra ? "  " + extra : ""}`); if (!ok) failures++; };

const BASE_NAMES = ["Doors open / walk-in music", "Host welcome", "Opening video", "Keynote speaker one", "Q&A", "Coffee break loop",
    "Panel discussion", "Award ceremony", "Sponsor reel", "Closing remarks", "Outro music", "Strike and load out"];
const NAMES = Array.from({ length: 30 }, (_, i) => BASE_NAMES[i % BASE_NAMES.length] + (i >= BASE_NAMES.length ? ` ${Math.floor(i / BASE_NAMES.length) + 1}` : ""));
const lines = NAMES.map((name, i) => ({ id: `l${i}`, position: `k${String(i + 1).padStart(6, "0")}V`, name,
    note: i % 3 === 0 ? "Wait for the host to reach the lectern before starting the next item." : "", duration: 60 + i * 95, absolute_time: null }));
const comments = [];
lines.forEach((l, i) => {
    if (i % 2 === 0) comments.push({ id: `c${i}a`, line_id: l.id, role_id: "r1", comment: "Fade house lights to 20%" });
    if (i % 3 === 0) comments.push({ id: `c${i}b`, line_id: l.id, role_id: "r2", comment: "Cam 2 on speaker, cam 1 wide" });
});
const details = { show: { id: "s1", name: "Annual Summit", time: 1800000000, role: "owner" },
    roles: [{ id: "r1", show_id: "s1", name: "Lighting" }, { id: "r2", show_id: "s1", name: "Video" }, { id: "r3", show_id: "s1", name: "Audio" }],
    lines, comments, rev: 4 };

const fakeScript = (profile, signedIn) => `
  window.__TAURI_INTERNALS__ = { transformCallback: () => 0, metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
    invoke: async (cmd, a) => {
      if (cmd === "platform_info") return { kind: ${JSON.stringify(profile)}, os: ${profile === "mobile" ? '"android"' : '"linux"'} };
      if (cmd === "auth_load") return { refresh_token: ${signedIn ? '"r"' : "null"}, refresh_expires_at: "2027", device_id: "d", grace_remaining_seconds: 9999, user_id: "u" };
      if (cmd === "get_shows") return [
        { id: "s1", name: "Annual Summit", time: 1800000000, role: "owner" },
        { id: "s2", name: "Product launch with a rather long name that has to wrap nicely", time: 1800100000, role: "editor" },
        { id: "s3", name: "Town hall", time: 1800200000, role: "viewer" }];
      if (cmd === "get_full_show_details") return ${JSON.stringify(details)};
      if (cmd === "sync_state") return { account: "u", shows: [], pending_total: 3 };
      if (cmd === "sync_pending") return [];
      if (cmd === "plugin:event|listen") return 1;
      return null; } };
  const json = b => ({ ok: true, status: 200, json: async () => b });
  window.fetch = async (url) => { url = String(url);
    if (url.endsWith("/members")) return json([{ user_id: "u", username: "laurens", role: "owner", is_me: true }, { user_id: "b", username: "bob-with-a-long-user-name", role: "editor", is_me: false }]);
    if (url.endsWith("/invites")) return json([{ id: "i1", role: "commenter", email: null, expires_at: "2026-12-02T10:00:00Z", max_uses: 10, used_count: 2 }]);
    if (url.includes("/auth/devices")) return json([{ id: "d1", platform: "mobile", app_version: "1.0", last_seen: "2026-11-25T10:00:00Z", current: true }]);
    return json({ access_token: "a", token_type: "Bearer", expires_in: 900, refresh_token: "r", refresh_expires_at: "2027-01-01T00:00:00Z",
      user: { id: "u", username: "laurens", email: "l@example.com", email_verified: false } }); };`;

async function open(device, signedIn) {
    const target = await (await fetch("http://127.0.0.1:9343/json/new?about:blank", { method: "PUT" })).json();
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise(r => ws.onopen = r);
    let id = 0; const pending = new Map(); const problems = [];
    ws.onmessage = ({ data }) => {
        const m = JSON.parse(data);
        if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id); return; }
        if (m.method === "Runtime.exceptionThrown") problems.push("EXC " + m.params.exceptionDetails.exception?.description?.split("\n")[0]);
    };
    const send = (method, params = {}) => new Promise(r => { const n = ++id; pending.set(n, r); ws.send(JSON.stringify({ id: n, method, params })); });
    const ev = async expression => {
        const r = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
        return r.exceptionDetails ? "EXC:" + (r.exceptionDetails.exception?.description ?? r.exceptionDetails.text) : r.result?.value;
    };
    for (const d of ["Runtime", "Page"]) await send(d + ".enable");
    await send("Emulation.setDeviceMetricsOverride", { width: device.width, height: device.height, deviceScaleFactor: device.touch ? 2 : 1, mobile: device.touch });
    await send("Emulation.setTouchEmulationEnabled", { enabled: device.touch, maxTouchPoints: device.touch ? 5 : 0 });
    await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
    await send("Page.addScriptToEvaluateOnNewDocument", { source: fakeScript(device.profile, signedIn) });
    return { send, ev, problems, ws,
        goto: async hash => { await send("Page.navigate", { url: `${BASE}/${hash}` }); await sleep(2200); },
        shot: async name => writeFileSync(join(OUT, `${device.name}-${name}.png`), Buffer.from((await send("Page.captureScreenshot", { format: "png" })).data, "base64")),
        tap: async selector => {
            const box = await ev(`(() => { const e = document.querySelector(${JSON.stringify(selector)}); e.scrollIntoView({ block: "center" }); const r = e.getBoundingClientRect(); return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 }); })()`);
            const { x, y } = JSON.parse(box);
            await send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
            await send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
            await sleep(250);
        },
        close: async () => { ws.close(); await fetch(`http://127.0.0.1:9343/json/close/${target.id}`); } };
}

// Everything measured in the page: overflow, dialog fit, touch target and font sizes.
const MEASURE = `(() => {
  const vw = window.innerWidth, vh = window.innerHeight;
  const visible = e => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return r.width > 0 && r.height > 0 && s.visibility !== "hidden" && s.display !== "none" && e.closest("[hidden]") === null; };
  const dialogs = [...document.querySelectorAll(".dialog-box")].filter(visible);
  const top = dialogs.at(-1) ?? document.body;
  const label = e => (e.tagName + "." + (e.getAttribute("class") || "").split(" ").slice(0, 2).join(".") + (e.getAttribute("title") ? "[" + e.getAttribute("title") + "]" : "") + " " + (e.textContent || "").trim().slice(0, 14)).trim();
  const controls = [...top.querySelectorAll("button, svg.btn-icon, a.auth-link, select, input:not([type=hidden]), textarea, .chip, .cal-day")].filter(visible)
    .filter(e => { const r = e.getBoundingClientRect(); return r.bottom > 0 && r.top < vh; });
  // Calendar days sit edge to edge in a seven column grid: they need the height, a little less width.
  const small = controls.filter(e => { const r = e.getBoundingClientRect();
    if (e.classList.contains("cal-day")) return r.height < 43.5 || r.width < 38;
    return Math.min(r.width, r.height) < 43.5 && !(e.tagName === "A" && Math.min(r.width, r.height) >= 24 && r.height >= 36); }).map(label);
  const tiny = [...top.querySelectorAll("input:not([type=hidden]), select, textarea")].filter(visible).filter(e => parseFloat(getComputedStyle(e).fontSize) < 15.99).map(label);
  const outside = dialogs.filter(d => { const r = d.getBoundingClientRect(); return r.left < -0.5 || r.top < -0.5 || r.right > vw + 0.5 || r.bottom > vh + 0.5; }).map(label);
  const wrapper = document.querySelector(".lines-table-wrapper");
  return JSON.stringify({ vw, vh, overflowX: document.documentElement.scrollWidth - vw, coarse: matchMedia("(pointer: coarse)").matches,
    small, tiny, outside, dialogs: dialogs.length,
    sheet: wrapper ? { scrolls: wrapper.scrollHeight > wrapper.clientHeight, pageScrolls: document.documentElement.scrollHeight > vh + 1 } : null });
})()`;

async function audit(page, device, screen, expect = {}) {
    await page.shot(screen);
    const m = JSON.parse(await page.ev(MEASURE));
    const tag = `${device.name} / ${screen}`;
    check(`${tag}: no horizontal overflow`, m.overflowX <= 1, m.overflowX > 1 ? `${m.overflowX}px too wide` : "");
    check(`${tag}: dialogs fit the screen`, m.outside.length === 0, m.outside.join(", "));
    if (device.touch) {
        check(`${tag}: touch emulation is active`, m.coarse);
        if (!expect.skipTouch) check(`${tag}: controls are at least 44px`, m.small.length === 0, m.small.slice(0, 6).join(" | "));
        check(`${tag}: form text is 16px or more`, m.tiny.length === 0, m.tiny.join(", "));
    }
    return m;
}

for (const device of DEVICES) {
    // signed out: login and register
    let page = await open(device, false);
    await page.goto("");
    await audit(page, device, "login");
    await page.goto("#register");
    await audit(page, device, "register");
    check(`${device.name}: no script errors while signed out`, page.problems.length === 0, page.problems.join(" | "));
    await page.close();

    // signed in
    page = await open(device, true);
    await page.goto("");
    check(`${device.name}: lands on the shows list`, (await page.ev("location.hash")) === "#home");
    await audit(page, device, "home");

    await page.ev(`document.querySelector(".account-menu svg").dispatchEvent(new MouseEvent("click", { bubbles: true }))`); await sleep(800);
    await audit(page, device, "account");
    await page.ev(`[...document.querySelectorAll(".box-container .close-btn")].at(-1).dispatchEvent(new MouseEvent("click", { bubbles: true }))`); await sleep(300);

    await page.ev(`document.getElementById("createShowBtn").dispatchEvent(new MouseEvent("click", { bubbles: true }))`); await sleep(600);
    await audit(page, device, "create-show");
    if (device.touch) {
        await page.tap(".cal-day:not(.is-outside):nth-of-type(20)");
        const picked = await page.ev(`document.querySelector(".cal-day.is-selected")?.textContent`);
        check(`${device.name}: a tap on the calendar picks the day`, picked !== undefined && picked !== null && picked !== "", String(picked));
    }
    await page.ev(`[...document.querySelectorAll(".modal-box .btn")].find(b => b.textContent === "Cancel")?.click()`); await sleep(400);

    await page.ev(`(async () => { const m = await import('/js/pages/edit_show.page.ts'); await m.openEditShow("s1"); })()`); await sleep(1200);
    const editor = await audit(page, device, "editor");
    check(`${device.name}: the run sheet scrolls inside the editor, not the page`, editor.sheet?.scrolls === true && editor.sheet?.pageScrolls === false, JSON.stringify(editor.sheet));
    const cards = await page.ev(`(() => { const r = document.querySelector(".lines-table tbody tr").getBoundingClientRect(); return JSON.stringify({ w: r.width, h: Math.round(r.height), titleW: Math.round(document.querySelector(".line-content-cell").getBoundingClientRect().width) }); })()`);
    console.log(`      first cue card: ${cards}`);

    await page.ev(`document.querySelector('.header-actions-group svg.btn-outline-primary').dispatchEvent(new MouseEvent("click", { bubbles: true }))`); await sleep(700);
    await audit(page, device, "add-cue");
    if (device.touch) {
        await page.tap(".chip");
        check(`${device.name}: a tap on a preset changes the duration`, (await page.ev(`document.getElementById("duration").value`)) !== "60");
    }
    await page.ev(`[...document.querySelectorAll(".modal-box .btn")].find(b => b.textContent === "Cancel")?.click()`); await sleep(400);

    await page.ev(`document.querySelector('.header-actions-group svg[title="Share"]').dispatchEvent(new MouseEvent("click", { bubbles: true }))`); await sleep(900);
    await audit(page, device, "share");
    check(`${device.name}: no script errors`, page.problems.length === 0, page.problems.join(" | "));
    await page.close();
}

console.log(failures ? `\n${failures} FAILED` : "\nall passed");
chrome.kill();
process.exit(failures ? 1 : 0);
