// Hosting a live session from the app: starting it, and the panel that manages a running one (the
// join QR code, who may join and call, requests, devices, callers). Everything here goes through
// IPC; nothing on the session's network socket can manage a session.
import { codeProblem, MAX_CODE_CHARS, MIN_CODE_CHARS } from "/shared/js/caller_code";
import { addHoverBox, dynamicPrompt, type ElementDefinition } from "/shared/js/dynamic_generator";
import type { AccessKind, AccessMethod } from "/shared/js/session_types";
import type { Show } from "/shared/js/show_types";
import type { SessionStatus } from "../api/types";
import { accessField, accessWarning, codeAttributes, gateCode, GATES, modeOf, requireHost, withMode, type AccessMode } from "./session_settings";

type Box = ReturnType<typeof addHoverBox>;

const METHOD_LABELS: Record<AccessMethod, string> = {
    open: "open access", code: "code", request: "approved", account: "account", host: "this computer"
};

/** Starts a session at once with the show's settings. Returns its id, or null if it failed. */
export async function startShowSession(show: Pick<Show, "id" | "name">): Promise<string | null> {
    try {
        return (await requireHost().startSession(show.id)).session_id;
    } catch (error) {
        await dynamicPrompt({
            title: `Could not start “${show.name}”`,
            confirmText: "OK",
            elements: [{ type: "p", text: error instanceof Error ? error.message : String(error) }]
        });
        return null;
    }
}

export interface PanelActions {
    refresh: () => Promise<void>;
    toggleCode: (kind: AccessKind) => void;
    close: () => void;
}

export function panelContent(status: SessionStatus, qr: string, shownCodes: ReadonlySet<AccessKind>, actions: PanelActions): ElementDefinition[] {
    const host = requireHost();
    const id = status.session_id;
    const { view } = status;
    const { settings } = view;
    const modes = { view: modeOf(settings.view), call: modeOf(settings.caller) };
    const then = (work: () => Promise<unknown>) => async () => {
        await work();
        await actions.refresh();
    };
    const row = (prefix: string, key: string): ElementDefinition => ({ type: "li", classes: "device-row", varId: `${prefix}_${key}`, target: `@${prefix}List` });
    const remove = (title: string, work: () => Promise<unknown>, target: string): ElementDefinition => ({
        type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], attributes: { title }, events: { click: then(work) }, target
    });

    const changeAccess = async (kind: AccessKind, mode: AccessMode, select: HTMLSelectElement): Promise<void> => {
        // Done with the dropdown: the panel may redraw now.
        select.blur?.();
        await host.saveSettings(status.show_id, withMode(settings, kind, mode));
        await actions.refresh();
    };

    const codeSection = (kind: AccessKind): ElementDefinition[] => {
        const code = gateCode(settings, kind);
        if (modes[kind] !== "code" || !code) return [];
        const shown = shownCodes.has(kind);
        const line = `${kind}CodeRow`;
        const paused = kind === "view" ? view.view_paused_seconds : view.call_paused_seconds;
        const chip = (text: string, click: () => unknown): ElementDefinition => ({
            type: "button", classes: "chip", text, attributes: { type: "button" }, events: { click }, target: `@${line}`
        });
        return [
            { type: "div", classes: "invite-link-row", varId: line, target: "@panel" },
            { type: "span", classes: "session-code", data: { gate: kind }, text: shown ? code : "•".repeat(Math.min(code.length, 16)), target: `@${line}` },
            chip(shown ? "Hide" : "Show", () => actions.toggleCode(kind)),
            chip("Change", then(() => changeCode(id, kind))),
            chip("Generate", then(() => generateCode(id, kind, code.length))),
            ...(paused ? [{ type: "p", classes: "modal-error-message", text: `Many wrong codes were tried: code entry is paused for ${paused} s.`, target: "@panel" }] : [])
        ];
    };

    const gate = (kind: AccessKind): ElementDefinition[] => [
        ...accessField("panel", kind, modes[kind], (mode, select) => { void changeAccess(kind, mode, select); }, "@panel"),
        ...(modes[kind] === "account" && !status.account_ready
            ? [{ type: "p", classes: "modal-error-message", text: "Account access does not work yet: this computer has to sync with the cloud once (and the server must offer it).", target: "@panel" }]
            : []),
        ...codeSection(kind)
    ];

    const warning = accessWarning(modes.view, modes.call);

    const requests: ElementDefinition[] = view.requests.length === 0 ? [] : [
        { type: "h3", text: "Waiting for approval", target: "@panel" },
        { type: "ul", classes: "device-list", varId: "requestList", target: "@panel" },
        ...view.requests.flatMap((request): ElementDefinition[] => {
            const key = `${request.client_id}_${request.kind}`;
            const answer = (approve: boolean) => then(() => host.answerRequest(id, request.client_id, request.kind, approve));
            return [
                row("request", key),
                { type: "span", classes: "device-name", text: request.name, target: `@request_${key}` },
                { type: "span", classes: "device-seen", text: request.kind === "view" ? "wants to join" : "wants to call", target: `@request_${key}` },
                { type: "button", classes: ["btn", "btn-primary"], text: "Approve", attributes: { type: "button" }, events: { click: answer(true) }, target: `@request_${key}` },
                { type: "button", classes: ["btn", "btn-outline-danger"], text: "Deny", attributes: { type: "button" }, events: { click: answer(false) }, target: `@request_${key}` }
            ];
        })
    ];

    // Who is in. Only a protected session can keep a removed device out.
    const inside = view.people.filter(person => person.admitted);
    const devices: ElementDefinition[] = modes.view === "open" || inside.every(person => person.is_host) ? [] : [
        { type: "h3", text: "Joined devices", target: "@panel" },
        { type: "ul", classes: "device-list", varId: "deviceList", target: "@panel" },
        ...inside.filter(person => !person.is_host).flatMap((person): ElementDefinition[] => [
            row("device", person.client_id),
            { type: "span", classes: "device-name", text: person.name ?? "Unnamed device", target: `@device_${person.client_id}` },
            ...(person.is_caller ? [{ type: "span", classes: "device-seen", text: "show caller", target: `@device_${person.client_id}` }] : []),
            remove("Remove from the session", () => host.removeDevice(id, person.client_id), `@device_${person.client_id}`)
        ])
    ];

    const callers: ElementDefinition[] = view.callers.length === 0
        ? [{ type: "small", classes: "empty-msg", text: "No show callers yet.", target: "@panel" }]
        : [
            { type: "ul", classes: "device-list", varId: "callerList", target: "@panel" },
            ...view.callers.flatMap((caller): ElementDefinition[] => [
                row("caller", caller.grant_id),
                { type: "span", classes: "device-name", text: caller.name, target: `@caller_${caller.grant_id}` },
                { type: "span", classes: "device-seen", text: `${METHOD_LABELS[caller.method]}${caller.online ? "" : " · offline"}`, target: `@caller_${caller.grant_id}` },
                ...(caller.method === "host" ? [] : [remove("Remove caller access", () => host.revokeCaller(id, caller.grant_id), `@caller_${caller.grant_id}`)])
            ])
        ];

    const connected = inside.length;

    return [
        { type: "div", classes: "account-body", varId: "panel" },
        // Requests come first: someone is waiting.
        ...requests,
        { type: "h3", text: "Join", target: "@panel" },
        { type: "div", classes: "session-qr", varId: "qrBox", target: "@panel" },
        { type: "svg", sketch: qr, target: "@qrBox" },
        { type: "div", classes: "invite-link-row", varId: "urlRow", target: "@panel" },
        { type: "input", id: "sessionUrl", attributes: { type: "text", readonly: "readonly", value: status.url }, target: "@urlRow" },
        { type: "button", classes: "chip", text: "Copy", attributes: { type: "button" }, events: { click: () => { void navigator.clipboard?.writeText(status.url); } }, target: "@urlRow" },
        { type: "p", classes: "account-email", text: `${connected} ${connected === 1 ? "device" : "devices"} connected${view.current_cue ? ` · on cue ${view.current_cue}` : ""}`, target: "@panel" },
        ...(status.discoverable
            ? []
            : [{ type: "p", classes: "modal-error-message", text: "Other apps cannot find this session by themselves (is another ShowFlow running here?). They can join by address.", target: "@panel" }]),

        { type: "h3", text: "Access", target: "@panel" },
        ...gate("view"),
        ...gate("call"),
        ...(warning ? [{ type: "p", classes: "modal-error-message", text: warning, target: "@panel" }] : []),
        ...devices,

        { type: "h3", text: "Show callers", target: "@panel" },
        ...callers,

        { type: "div", classes: "modal-footer", varId: "panelFooter", target: "@panel" },
        {
            type: "button", classes: ["btn", "btn-outline-danger"], text: "End session", attributes: { type: "button" }, target: "@panelFooter",
            events: { click: async () => { if (await confirmEnd(status.show_name)) { await host.stopSession(id); actions.close(); } } }
        },
        {
            type: "button", classes: ["btn", "btn-primary"], text: "Call from this computer", attributes: { type: "button" }, target: "@panelFooter",
            events: { click: () => { void host.openCallerWindow(id); } }
        }
    ];
}

async function confirmEnd(showName: string): Promise<boolean> {
    const answer = await dynamicPrompt({
        title: "End session",
        confirmText: "End session",
        elements: [{ type: "p", text: `Everyone connected to "${showName}" is disconnected.` }]
    });
    return answer !== null;
}

async function changeCode(sessionId: string, kind: AccessKind): Promise<void> {
    const host = requireHost();
    await dynamicPrompt({
        title: `Change the ${GATES[kind].codeLabel.toLowerCase()}`,
        confirmText: "Change",
        elements: [
            { type: "p", classes: "account-email", text: "Whoever came in with the old code loses that access." },
            { type: "input-label", id: "newCode", label: `New code (${MIN_CODE_CHARS}–${MAX_CODE_CHARS} characters)`, attributes: codeAttributes }
        ],
        onValidate: async (data): Promise<Record<string, string> | null> => {
            const problem = codeProblem(String(data.newCode ?? ""));
            if (problem) return { newCode: problem };
            await host.setCode(sessionId, kind, String(data.newCode));
            return null;
        }
    });
}

async function generateCode(sessionId: string, kind: AccessKind, currentLength: number): Promise<void> {
    const host = requireHost();
    await dynamicPrompt({
        title: `Generate a new ${GATES[kind].codeLabel.toLowerCase()}`,
        confirmText: "Generate",
        elements: [
            { type: "p", classes: "account-email", text: "A random hexadecimal code. Whoever came in with the old code loses that access." },
            { type: "input-label", id: "codeLength", label: `Length (${MIN_CODE_CHARS}–${MAX_CODE_CHARS})`, inputType: "number", value: Math.min(Math.max(currentLength, MIN_CODE_CHARS), MAX_CODE_CHARS), attributes: { min: MIN_CODE_CHARS, max: MAX_CODE_CHARS } }
        ],
        onValidate: async (data): Promise<Record<string, string> | null> => {
            const length = Number(data.codeLength);
            if (!Number.isInteger(length) || length < MIN_CODE_CHARS || length > MAX_CODE_CHARS) {
                return { codeLength: `Choose a length from ${MIN_CODE_CHARS} to ${MAX_CODE_CHARS}` };
            }
            await host.setCode(sessionId, kind, null, length);
            return null;
        }
    });
}

/** Opens the management panel of a running session; it follows the session live. */
export async function openSessionPanel(sessionId: string): Promise<void> {
    const host = requireHost();
    const [status, qr] = await Promise.all([host.status(sessionId), host.qr(sessionId)]);
    const shownCodes = new Set<AccessKind>();
    let box: Box;
    let stopWatching: (() => void) | null = null;
    const isOpen = () => box.container.isConnected !== false && box.container.style.display !== "none";
    const close = (): void => { stopWatching?.(); stopWatching = null; box.close(); };
    const render = async (): Promise<void> => {
        let latest: SessionStatus;
        try {
            latest = await host.status(sessionId);
        } catch {
            close(); // the session ended
            return;
        }
        // Redrawing would snap an open dropdown shut (someone joining is enough to get here).
        const busy = document.activeElement;
        if (busy?.tagName === "SELECT" && busy.id.startsWith("panel-")) {
            busy.addEventListener("blur", () => { void render(); }, { once: true });
            return;
        }
        box.refreshContent(panelContent(latest, qr, shownCodes, actions));
    };
    const actions: PanelActions = {
        refresh: render, close,
        toggleCode: kind => {
            if (!shownCodes.delete(kind)) shownCodes.add(kind);
            void render();
        }
    };

    box = addHoverBox(null, `Session — ${status.show_name}`, panelContent(status, qr, shownCodes, actions), { width: "560px", height: "auto" });
    stopWatching = await host.onSessionUpdated(updated => {
        if (updated !== sessionId) return;
        if (isOpen()) void render();
        else { stopWatching?.(); stopWatching = null; }
    });
}
