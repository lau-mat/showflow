import { addHoverBox, generateDynamic, type ElementDefinition } from "/shared/js/dynamic_generator";
import { getPlatform } from "/shared/js/platform";
import { getApi } from "../api";
import type { Device } from "../api/types";
import type { AuthSession } from "../auth/session";
import type { SyncStatus, SyncStatusStore } from "../sync/status";
import { loadPage } from "../page_loader";

const PLATFORM_LABELS: Record<Device["platform"], string> = { web: "Web browser", desktop: "Desktop app", mobile: "Mobile app" };

type Box = ReturnType<typeof addHoverBox>;

/** `devices` is null when they could not be loaded (offline, server down). */
function accountContent(session: AuthSession, devices: Device[] | null, notice: string | null, box: () => Box): ElementDefinition[] {
    const refresh = () => refreshDevices(session, box());
    const deviceRows: ElementDefinition[] = (devices ?? []).flatMap(device => [
        { type: "li", classes: "device-row", varId: `device_${device.id}`, target: "@deviceList" },
        {
            type: "span", classes: "device-name", target: `@device_${device.id}`,
            text: `${PLATFORM_LABELS[device.platform]}${device.app_version && device.platform !== "web" ? ` ${device.app_version}` : ""}`
        },
        { type: "span", classes: "device-seen", text: device.current ? "This device" : new Date(device.last_seen).toLocaleString(), target: `@device_${device.id}` },
        ...(device.current ? [] : [{
            type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], attributes: { title: "Log this device out" },
            events: { click: async () => { await getApi().auth.revokeDevice(device.id); await refresh(); } },
            target: `@device_${device.id}`
        }])
    ]);

    return [
        { type: "div", classes: "account-body", varId: "accountBody" },
        { type: "h3", classes: "account-name", text: session.user?.username ?? "Account", target: "@accountBody" },
        { type: "p", classes: "account-email", text: session.user?.email ?? "", target: "@accountBody" },
        ...(notice ? [{ type: "p", classes: "modal-error-message", text: notice, target: "@accountBody" }] : []),
        ...(session.state === "offlineGrace"
            ? [{ type: "p", classes: "account-email", text: "Offline: your data stays available until you reconnect.", target: "@accountBody" }]
            : []),
        { type: "h3", text: "Logged in on", target: "@accountBody" },
        devices === null
            ? { type: "small", classes: "empty-msg", text: "Devices are unavailable while offline.", target: "@accountBody" }
            : { type: "ul", classes: "device-list", varId: "deviceList", target: "@accountBody" },
        ...deviceRows,
        { type: "div", classes: "modal-footer", varId: "accountFooter", target: "@accountBody" },
        { type: "button", classes: ["btn", "btn-outline-danger"], text: "Log out everywhere", events: { click: () => leave(session, box(), true) }, target: "@accountFooter" },
        { type: "button", classes: ["btn", "btn-primary"], text: "Log out", events: { click: () => leave(session, box(), false) }, target: "@accountFooter" }
    ];
}

async function loadDevices(): Promise<Device[] | null> {
    try {
        return await getApi().auth.listDevices();
    } catch {
        return null;
    }
}

async function refreshDevices(session: AuthSession, box: Box, notice: string | null = null): Promise<void> {
    const devices = await loadDevices();
    box.refreshContent(accountContent(session, devices, notice, () => box));
}

async function leave(session: AuthSession, box: Box, everywhere: boolean): Promise<void> {
    try {
        await (everywhere ? session.logoutEverywhere() : session.logout());
    } catch {
        await refreshDevices(session, box, "Could not reach the server to log out. Try again when you are online.");
        return;
    }
    box.close();
    await loadPage(getPlatform().loginPage);
}

export async function openAccountBox(session: AuthSession): Promise<void> {
    const devices = await loadDevices();
    let box: Box;
    box = addHoverBox(null, "Account", accountContent(session, devices, null, () => box), { width: "460px", height: "auto" });
}

/** Short text for the sync badge; empty hides it. */
export function syncBadgeText(status: SyncStatus): string {
    const pending = status.pending > 0 ? `${status.pending} unsynced` : "";
    switch (status.phase) {
        case "syncing": return "Syncing…";
        case "offline": return pending ? `Offline · ${pending}` : "Offline";
        case "error": return "Sync problem";
        case "paused": return pending;
        default: return pending || "Synced";
    }
}

/** The account button in the corner (with the sync badge in the apps): only visible while signed in. */
export function mountAccountMenu(session: AuthSession, syncStatus: SyncStatusStore | null = null): void {
    const { menu, badge } = generateDynamic([
        { type: "div", classes: "account-menu", varId: "menu" },
        { type: "span", classes: "sync-badge", varId: "badge", target: "@menu" },
        { type: "svg", file: "icons/user.svg", classes: ["btn-icon", "btn-muted"], attributes: { title: "Account" }, events: { click: () => { void openAccountBox(session); } }, target: "@menu" }
    ], document.body);
    const sync = () => { (menu as HTMLElement).hidden = !session.isAuthorized; };
    session.subscribe(sync);
    sync();

    const paint = (status: SyncStatus) => {
        const text = syncBadgeText(status);
        const element = badge as HTMLElement;
        element.textContent = text;
        element.hidden = syncStatus === null || text === "";
        element.dataset.phase = status.phase;
        element.title = status.message ?? (status.lastSyncedAt ? `Last synced ${new Date(status.lastSyncedAt).toLocaleTimeString()}` : "");
    };
    if (syncStatus) {
        syncStatus.subscribe(paint);
        paint(syncStatus.status);
    } else {
        (badge as HTMLElement).hidden = true;
    }
}
