import { invoke } from "@tauri-apps/api/core";

export type PlatformKind = "mobile" | "desktop" | "web";

export type Feature =
    | "showEditor"
    | "sessionView"
    | "sessionHost"
    | "sessionDiscovery"
    | "qrShare"
    | "fullscreenWindow"
    | "print"
    | "offlineData";

export interface PlatformProfile {
    kind: PlatformKind;
    /** linux, windows, macos, android, ios, or "web". */
    os: string;
    /** How data is reached: Tauri IPC (local sqlite) or the cloud REST api. */
    transport: "ipc" | "rest";
    /** Days the app may run offline after the last successful online validation (null: no offline use). */
    offlineGraceDays: number | null;
    loginPage: string;
    registerPage: string;
    landingPage: string;
    /**
     * Where the long-lived login token lives. Browsers: an HttpOnly cookie the server sets (JS
     * never sees it). Apps: the local app database, so the login survives restarts offline.
     */
    tokenStorage: "cookie" | "device";
    features: Record<Feature, boolean>;
}

type ProfileDefinition = Omit<PlatformProfile, "kind" | "os">;

const features = (enabled: Feature[]): Record<Feature, boolean> => {
    const all: Feature[] = [
        "showEditor", "sessionView", "sessionHost", "sessionDiscovery",
        "qrShare", "fullscreenWindow", "print", "offlineData"
    ];
    return Object.fromEntries(all.map(feature => [feature, enabled.includes(feature)])) as Record<Feature, boolean>;
};

/** Every per-platform difference lives in this table. */
export const PROFILES: Record<PlatformKind, ProfileDefinition> = {
    desktop: {
        transport: "ipc",
        offlineGraceDays: 14,
        loginPage: "login",
        registerPage: "register",
        landingPage: "home",
        tokenStorage: "device",
        features: features([
            "showEditor", "sessionView", "sessionHost", "sessionDiscovery",
            "qrShare", "fullscreenWindow", "print", "offlineData"
        ])
    },
    mobile: {
        transport: "ipc",
        offlineGraceDays: 14,
        loginPage: "login",
        registerPage: "register",
        landingPage: "sessions",
        tokenStorage: "device",
        features: features(["showEditor", "sessionView", "sessionDiscovery", "offlineData"])
    },
    web: {
        transport: "rest",
        offlineGraceDays: null,
        loginPage: "login",
        registerPage: "register",
        landingPage: "home",
        tokenStorage: "cookie",
        features: features(["showEditor", "sessionView", "print"])
    }
};

interface NativePlatformInfo {
    kind: "desktop" | "mobile";
    os: string;
}

let current: PlatformProfile | null = null;
let pending: Promise<PlatformProfile> | null = null;

export function isTauriRuntime(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function buildProfile(kind: PlatformKind, os: string): PlatformProfile {
    return { kind, os, ...PROFILES[kind] };
}

/** Works out the runtime profile once; later calls return the same result. */
export function detectPlatform(): Promise<PlatformProfile> {
    pending ??= (async () => {
        if (!isTauriRuntime()) return buildProfile("web", "web");
        const info = await invoke<NativePlatformInfo>("platform_info");
        return buildProfile(info.kind, info.os);
    })().then(profile => (current = profile));
    // A failed detection must not be cached.
    pending.catch(() => { pending = null; });
    return pending;
}

export function installPlatform(profile: PlatformProfile | null): void {
    current = profile;
    pending = profile ? Promise.resolve(profile) : null;
}

export function getPlatform(): PlatformProfile {
    if (!current) throw new Error("Platform not detected yet; await detectPlatform() first.");
    return current;
}

export function hasFeature(feature: Feature): boolean {
    return getPlatform().features[feature];
}
