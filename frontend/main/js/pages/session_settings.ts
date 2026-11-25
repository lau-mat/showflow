// Who may join a show's sessions and who may call them: the two dropdowns, and the dialog behind
// the settings button in the show editor. The panel of a running session uses the same dropdowns.
import "/shared/js/compound_components";
import { codeProblem, MAX_CODE_CHARS, MIN_CODE_CHARS } from "/shared/js/caller_code";
import { dynamicPrompt, type ElementDefinition } from "/shared/js/dynamic_generator";
import type { AccessKind, AccessPolicy, AccessSettings } from "/shared/js/session_types";
import type { Show } from "/shared/js/show_types";
import { getApi } from "../api";
import type { HostApi } from "../api/types";

/** The one choice per dropdown; `host` lets in nobody but this computer. */
export type AccessMode = "request" | "code" | "account" | "open" | "host";

export const MODE_LABELS: Record<AccessMode, string> = {
    open: "Open to everyone",
    request: "On approval",
    code: "With a code",
    account: "By account",
    host: "Only this computer"
};

interface GateText {
    label: string;
    codeLabel: string;
    /** Listed in this order: the default comes first. */
    modes: AccessMode[];
    hints: Record<AccessMode, string>;
}

export const GATES: Record<AccessKind, GateText> = {
    view: {
        label: "Who can join",
        codeLabel: "Access code",
        modes: ["open", "request", "code", "account", "host"],
        hints: {
            open: "Anyone who can reach this computer can follow the show.",
            request: "You approve each device before it sees anything.",
            code: "Devices need the access code before they see anything.",
            account: "Only ShowFlow apps of people with access to this show (see Share).",
            host: "No other device can join."
        }
    },
    call: {
        label: "Who can become show caller",
        codeLabel: "Caller code",
        modes: ["request", "code", "account", "open", "host"],
        hints: {
            request: "You approve each device that asks to call the show.",
            code: "Anyone who has joined and knows the caller code can run the show.",
            account: "ShowFlow apps of people marked “Show caller” under Share.",
            open: "Anyone who has joined can run the show.",
            host: "Only this computer can run the show."
        }
    }
};

const KINDS: AccessKind[] = ["view", "call"];

export function policyFor(mode: AccessMode): AccessPolicy {
    return { open: mode === "open", code: mode === "code", request: mode === "request", account: mode === "account" };
}

export function modeOf(policy: AccessPolicy): AccessMode {
    return (["open", "code", "account", "request"] as const).find(mode => policy[mode]) ?? "host";
}

const policyKey = { view: "view", call: "caller" } as const;
const codeKey = { view: "view_code", call: "caller_code" } as const;

export const gatePolicy = (settings: AccessSettings, kind: AccessKind): AccessPolicy => settings[policyKey[kind]];
export const gateCode = (settings: AccessSettings, kind: AccessKind): string | null => settings[codeKey[kind]];

export function withMode(settings: AccessSettings, kind: AccessKind, mode: AccessMode): AccessSettings {
    return { ...settings, [policyKey[kind]]: policyFor(mode) };
}

export const codeAttributes = { autocapitalize: "off", autocorrect: "off", autocomplete: "off", spellcheck: "false", maxlength: MAX_CODE_CHARS };

export function requireHost(): HostApi {
    const { host } = getApi();
    if (!host) throw new Error("This device cannot host sessions.");
    return host;
}

/**
 * One gate's dropdown with a line under it saying what the choice means. The dropdown is
 * `<prefix>-<kind>`, the line `<prefix>-<kind>-hint`.
 */
export function accessField(prefix: string, kind: AccessKind, mode: AccessMode, change: (mode: AccessMode, select: HTMLSelectElement) => void, target?: string): ElementDefinition[] {
    const gate = GATES[kind];
    const id = `${prefix}-${kind}`;
    const placed = target ? { target } : {};
    return [
        {
            type: "select-label", id, label: gate.label, classes: "access-field",
            options: gate.modes.map(option => ({
                type: "option", text: MODE_LABELS[option],
                attributes: { value: option, ...(option === mode ? { selected: "selected" } : {}) }
            })),
            events: {
                change: (event: Event) => {
                    const select = event.target as HTMLSelectElement;
                    const chosen = select.value as AccessMode;
                    const hint = document.getElementById(`${id}-hint`);
                    if (hint) hint.textContent = gate.hints[chosen];
                    change(chosen, select);
                }
            },
            ...placed
        },
        { type: "p", id: `${id}-hint`, classes: "access-hint", text: gate.hints[mode], ...placed }
    ];
}

/** Closing the door to others also leaves nobody to call. */
export function accessWarning(view: AccessMode, call: AccessMode): string {
    return view === "host" && call !== "host" ? "Nobody else can join, so nobody else can call either." : "";
}

/**
 * The settings dialog's values as settings. A code is only taken over for a gate that uses one;
 * otherwise the remembered one is kept for next time. Empty means "generate one".
 */
export function settingsFrom(values: Record<string, unknown>, previous: AccessSettings): AccessSettings {
    return KINDS.reduce((settings, kind) => {
        const mode = values[`settings-${kind}`] as AccessMode;
        const next = withMode(settings, kind, mode);
        return mode === "code" ? { ...next, [codeKey[kind]]: String(values[`settings-${kind}-code`] ?? "") || null } : next;
    }, previous);
}

/** The settings behind the editor's settings button. A running session follows at once. */
export async function openSessionSettings(show: Pick<Show, "id" | "name">): Promise<void> {
    const host = requireHost();
    const settings = await host.settings(show.id);
    const modes = { view: modeOf(settings.view), call: modeOf(settings.caller) };

    const showWarning = (): void => {
        const warning = document.getElementById("settings-warning");
        if (warning) warning.textContent = accessWarning(modes.view, modes.call);
    };

    const gate = (kind: AccessKind): ElementDefinition[] => [
        ...accessField("settings", kind, modes[kind], mode => {
            modes[kind] = mode;
            const group = document.getElementById(`settings-${kind}-code-group`);
            if (group) group.style.display = mode === "code" ? "" : "none";
            showWarning();
        }),
        {
            type: "input-label", id: `settings-${kind}-code`,
            label: `${GATES[kind].codeLabel} (${MIN_CODE_CHARS}–${MAX_CODE_CHARS} characters, empty to generate one)`,
            value: gateCode(settings, kind) ?? "", attributes: codeAttributes,
            style: modes[kind] === "code" ? {} : { display: "none" }
        }
    ];

    await dynamicPrompt({
        title: `Session settings — ${show.name}`,
        confirmText: "Save",
        elements: [
            ...gate("view"),
            ...gate("call"),
            { type: "p", id: "settings-warning", classes: "modal-error-message", text: accessWarning(modes.view, modes.call) }
        ],
        onValidate: async (data): Promise<Record<string, string> | null> => {
            const next = settingsFrom(data, settings);
            const errors: Record<string, string> = {};
            for (const kind of KINDS) {
                const code = gateCode(next, kind);
                const problem = data[`settings-${kind}`] === "code" && code ? codeProblem(code) : null;
                if (problem) errors[`settings-${kind}-code`] = problem;
            }
            if (Object.keys(errors).length > 0) return errors;
            try {
                await host.saveSettings(show.id, next);
            } catch (error) {
                return { "settings-view": error instanceof Error ? error.message : String(error) };
            }
            return null;
        }
    });
}
