// Getting in and becoming show caller. A session has two gates with the same choices: joining
// (seeing the show) and calling (running it). What asking does depends on how the host set the
// gate up: open passes at once; the ShowFlow app of someone with the right account is let in by
// its pass; otherwise a code is asked for first, or a request waits for the host.
import { invoke } from "@tauri-apps/api/core";
import "/shared/js/compound_components";
import { dynamicGenerator, dynamicPrompt, generateDynamic, type ElementDefinition } from "/shared/js/dynamic_generator";
import { codeProblem, MAX_CODE_CHARS } from "/shared/js/caller_code";
import { resolveSessionAddress, SessionSocket } from "/shared/js/session_socket";
import type { AccessKind, AccessPolicy, Claim, ServerMessages } from "/shared/js/session_types";

const resumeKey = (kind: AccessKind, sessionId: string) => `showflow.${kind === "call" ? "caller" : "view"}.${sessionId}`;
const REASON_KEY = "showflow.caller_notice";

/** What lets this device back in after a reload or a Wi-Fi drop, per gate and session. */
export const resumeToken = {
    load: (kind: AccessKind, sessionId: string): string | null => window.localStorage.getItem(resumeKey(kind, sessionId)),
    save: (kind: AccessKind, sessionId: string, token: string): void => window.localStorage.setItem(resumeKey(kind, sessionId), token),
    clear: (kind: AccessKind, sessionId: string): void => window.localStorage.removeItem?.(resumeKey(kind, sessionId))
};

/** A message to show on the next page (why the caller view sent you back). */
export const pageNotice = {
    set: (text: string): void => globalThis.sessionStorage?.setItem(REASON_KEY, text),
    take(): string | null {
        const text = globalThis.sessionStorage?.getItem(REASON_KEY) ?? null;
        globalThis.sessionStorage?.removeItem(REASON_KEY);
        return text;
    }
};

/** The pages run inside the ShowFlow app (not in a plain browser). */
const insideApp = (): boolean => "__TAURI_INTERNALS__" in window;

type SessionChallenge = Pick<ServerMessages["welcome"], "session_id" | "show_id" | "nonce">;

/**
 * The app's answer to this host's challenge, if the signed-in account holds a caller pass for the
 * show. Browsers never do.
 */
export async function appAccountClaim(challenge: SessionChallenge): Promise<Claim | null> {
    if (!insideApp()) return null;
    try {
        const answer = await invoke<{ pass: string; proof: string } | null>("caller_claim", {
            showId: challenge.show_id, sessionId: challenge.session_id, nonce: challenge.nonce
        });
        return answer ? { method: "account", pass: answer.pass, proof: answer.proof } : null;
    } catch (error) {
        console.error("Could not use the caller pass:", error);
        return null;
    }
}

/** How each gate words the same questions. */
const WORDING = {
    view: {
        title: "Join the session",
        code: "Access code",
        closed: "The host has not opened this session to other devices.",
        browser: "Only signed-in ShowFlow apps of people with access to this show can join.",
        noPass: "This app has no pass for this show. Your account needs access to the show, and the app has to be online once after that."
    },
    call: {
        title: "Become show caller",
        code: "Caller code",
        closed: "The host has not opened the caller role to other devices.",
        browser: "Only signed-in ShowFlow apps that the owner made a caller can call this show.",
        noPass: "This app has no caller pass for this show. The owner has to mark your account as “Show caller” under Share, and the app has to be online once after that."
    }
} as const;

/**
 * Collects what the claim needs. Open access asks nothing. Returns null when the user cancels,
 * or when this device has no way in (the user is told).
 */
export async function askClaim(
    kind: AccessKind,
    policy: AccessPolicy,
    name: string,
    prompt: typeof dynamicPrompt = dynamicPrompt,
    inApp: boolean = insideApp()
): Promise<Claim | null> {
    if (policy.open) return { method: "open", name };
    const words = WORDING[kind];

    if (!policy.code && !policy.request) {
        await prompt({
            title: words.title,
            confirmText: "OK",
            elements: [{ type: "p", text: !policy.account ? words.closed : inApp ? words.noPass : words.browser }]
        });
        return null;
    }

    const codeField: ElementDefinition[] = policy.code ? [{
        type: "input-label", id: "accessCode", label: words.code, inputType: "password",
        attributes: { autocapitalize: "off", autocorrect: "off", autocomplete: "off", spellcheck: "false", maxlength: MAX_CODE_CHARS }
    }, {
        type: "button", classes: ["chip", "show-code"], text: "Show code", attributes: { type: "button" },
        events: {
            click: (event: Event) => {
                const input = document.getElementById("accessCode") as HTMLInputElement | null;
                if (!input) return;
                const hidden = input.type === "password";
                input.type = hidden ? "text" : "password";
                (event.currentTarget as HTMLElement).textContent = hidden ? "Hide code" : "Show code";
            }
        }
    }] : [];

    const answer = await prompt({
        title: words.title,
        confirmText: policy.code ? "Unlock" : "Ask the host",
        elements: [{ type: "input-label", id: "accessName", label: "Your name", value: name }, ...codeField],
        onValidate: data => {
            const errors: Record<string, string> = {};
            if (!String(data.accessName ?? "").trim()) errors.accessName = "Please fill in your name";
            if (policy.code) {
                const problem = codeProblem(String(data.accessCode ?? ""));
                if (problem) errors.accessCode = problem;
            }
            return errors;
        }
    });
    if (!answer) return null;
    const typedName = String(answer.accessName).trim();
    return policy.code
        ? { method: "code", name: typedName, code: String(answer.accessCode) }
        : { method: "request", name: typedName };
}

export interface AccessFlowDeps {
    /** Which gate this conversation is about. */
    kind: AccessKind;
    send: (claim: Claim) => void;
    /** The device is through. */
    onGranted: () => void;
    /** It is out (revoked, released, or a stored token no longer works). */
    onLost: (reason: string | null) => void;
    /** Progress and problems that do not change where the device stands. */
    notify: (text: string) => void;
    ask?: (kind: AccessKind, policy: AccessPolicy, name: string) => Promise<Claim | null>;
    account?: (challenge: SessionChallenge) => Promise<Claim | null>;
}

/** The claim conversation with the host about one gate, from asking to granted or lost. */
export class AccessFlow {
    private readonly deps: AccessFlowDeps;
    private challenge: SessionChallenge | null = null;
    /** The host turned the account claim down: the next press goes another way. */
    private accountRefused = false;
    policy: AccessPolicy = { open: false, code: false, request: false, account: false };
    granted = false;
    /** What the last claim was, to tell a stale stored token from a wrong code. */
    private lastMethod: Claim["method"] | null = null;

    constructor(deps: AccessFlowDeps) {
        this.deps = deps;
    }

    private get sessionId(): string | null {
        return this.challenge?.session_id ?? null;
    }

    welcome(message: SessionChallenge & Pick<ServerMessages["welcome"], "policy" | "view_policy">): void {
        this.challenge = message;
        this.adopt(message);
    }

    /** The host's rules, as announced in `welcome` and changed by `policy`. */
    adopt(rules: Pick<ServerMessages["welcome"], "policy" | "view_policy">): void {
        this.policy = this.deps.kind === "view" ? rules.view_policy : rules.policy;
    }

    /** Presents a stored token, if any; returns whether it did. */
    resume(): boolean {
        const token = this.sessionId ? resumeToken.load(this.deps.kind, this.sessionId) : null;
        if (!token) return false;
        this.claim({ method: "resume", token });
        return true;
    }

    claim(claim: Claim): void {
        this.lastMethod = claim.method;
        this.deps.send(claim);
    }

    async request(name: string): Promise<void> {
        if (this.policy.account && !this.accountRefused && this.challenge) {
            // A pass needs no typing, so it is always tried first.
            const claim = await (this.deps.account ?? appAccountClaim)(this.challenge);
            if (claim) {
                this.claim(claim);
                return;
            }
        }
        const claim = await (this.deps.ask ?? askClaim)(this.deps.kind, this.policy, name);
        if (claim) this.claim(claim);
    }

    handle(status: ServerMessages["caller_status"]): void {
        const { kind } = this.deps;
        switch (status.state) {
            case "granted":
                this.granted = true;
                if (this.sessionId && status.resume_token) resumeToken.save(kind, this.sessionId, status.resume_token);
                this.deps.onGranted();
                return;
            case "pending":
                this.deps.notify("Waiting for the host to approve…");
                return;
            case "denied":
                if (this.lastMethod === "resume") {
                    if (this.sessionId) resumeToken.clear(kind, this.sessionId);
                    this.deps.onLost(status.reason);
                    return;
                }
                if (this.lastMethod === "account") this.accountRefused = true;
                this.deps.notify(status.reason ?? "Not allowed.");
                return;
            case "revoked":
            case "released":
                this.granted = false;
                if (this.sessionId) resumeToken.clear(kind, this.sessionId);
                this.deps.onLost(status.reason);
        }
    }
}

/**
 * What a device sees of a protected session until it is let in: nothing about the show, only the
 * way to ask.
 */
export function showGate(onAsk: () => void): void {
    if (document.getElementById("viewGate")) return;
    generateDynamic([
        { type: "div", id: "viewGate", classes: "view-gate", varId: "gate" },
        { type: "div", classes: "view-gate-card", target: "@gate", varId: "card" },
        { type: "h1", text: "This session is protected", target: "@card" },
        { type: "p", id: "viewGateStatus", text: "The host decides who can join.", attributes: { role: "status" }, target: "@card" },
        { type: "button", classes: ["btn", "btn-primary"], text: "Join session", attributes: { type: "button" }, events: { click: onAsk }, target: "@card" }
    ], document.body);
}

/** Puts a message on the gate, or on top of the page when there is no gate. */
export function gateStatus(text: string): void {
    const status = document.getElementById("viewGateStatus");
    if (status) status.textContent = text;
    else showNotice(text);
}

export function hideGate(): void {
    document.getElementById("viewGate")?.remove();
}

/** A short message at the top of the page that fades out by itself. */
export function showNotice(text: string, durationMs = 5000): void {
    document.querySelector(".session-notice")?.remove();
    const notice = dynamicGenerator({ type: "div", classes: "session-notice", text, attributes: { role: "status" } }, document.body);
    if (durationMs > 0) setTimeout(() => notice.remove(), durationMs);
}

/**
 * Wi-Fi drops and restarted sessions: say so and reload once the session answers again. A reload
 * is the simplest way back to a consistent state; a caller resumes with the stored token.
 */
export function reconnectWhenPossible(check: () => Promise<unknown>, reload: () => void = () => window.location.reload()): void {
    showNotice("Connection lost. Reconnecting… (the session may have ended)", 0);
    const attempt = (): void => {
        check().then(reload, () => { setTimeout(attempt, 3000); });
    };
    setTimeout(attempt, 1500);
}

/** The app opened its own copy of this page for a session elsewhere: offer the way back. */
export function mountAppBackLink(search: string = window.location.search): boolean {
    if (!new URLSearchParams(search).has("host")) return false;
    dynamicGenerator({ type: "a", classes: "app-back", text: "← ShowFlow", attributes: { href: "../index.html" } }, document.body);
    return true;
}

/** Connects to the session this page belongs to, and handles the connection dropping later. */
export async function openSessionSocket(): Promise<SessionSocket> {
    mountAppBackLink();
    let socket: SessionSocket;
    try {
        socket = await SessionSocket.open();
    } catch (error) {
        showNotice(error instanceof Error ? error.message : "Could not reach the session.", 0);
        throw error;
    }
    socket.onClose(() => reconnectWhenPossible(() => resolveSessionAddress()));
    return socket;
}
