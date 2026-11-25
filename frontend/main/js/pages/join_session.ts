// Sessions on the network, in the apps. They are found by themselves: the home page lists what is
// running nearby and keeps the list current. Typing an address stays possible behind a button, for
// networks that block the search.
import { dynamicPrompt, generateDynamic, type ElementDefinition } from "/shared/js/dynamic_generator";
import { formatTarget, joinPagePath, parseJoinLink, type JoinTarget } from "/shared/js/join_link";
import { ProtocolMismatch, resolveSessionAddress } from "/shared/js/session_socket";
import { getApi } from "../api";
import type { FoundSession } from "../api/types";

const LAST_KEY = "showflow.last_session_address";
/** Pause between searches (a search itself listens for about a second). */
const SEARCH_EVERY_MS = 3000;

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">;

/** The address last joined by hand, to fill in again. */
export function lastAddress(storage: Storage = window.localStorage): string {
    const stored = storage.getItem(LAST_KEY) ?? "";
    return parseJoinLink(stored) ? stored : "";
}

export interface JoinDeps {
    /** Checks a session answers there; throws with a reason otherwise. */
    probe: (target: JoinTarget) => Promise<unknown>;
    open: (path: string) => void;
}

const defaultDeps: JoinDeps = {
    probe: target => resolveSessionAddress({ hostname: "", search: `?host=${encodeURIComponent(target.host)}&port=${target.port}` }, undefined),
    open: path => window.location.assign(path)
};

/** Opens the session at `input`, or says what is wrong with it. */
export async function joinSession(input: string, deps: JoinDeps = defaultDeps): Promise<string | null> {
    const target = parseJoinLink(input);
    if (!target) return "Enter the session's address, like 192.168.1.5:47123.";
    try {
        await deps.probe(target);
    } catch (error) {
        if (error instanceof ProtocolMismatch) return error.message;
        return "No ShowFlow session answers at that address. Check that both devices are on the same network.";
    }
    deps.open(joinPagePath(target));
    return null;
}

export async function openManualJoin(deps: JoinDeps = defaultDeps, storage: Storage = window.localStorage): Promise<void> {
    await dynamicPrompt({
        title: "Join by address",
        confirmText: "Join",
        elements: [
            { type: "p", classes: "account-email", text: "For networks that stop sessions from being found. The address is shown on the host's session panel." },
            {
                type: "input-label", id: "joinAddress", label: "Session address", value: lastAddress(storage),
                attributes: { placeholder: "192.168.1.5:47123", autocapitalize: "off", autocorrect: "off", spellcheck: "false", inputmode: "url" }
            }
        ],
        onValidate: async (data): Promise<Record<string, string> | null> => {
            const input = String(data.joinAddress ?? "");
            const problem = await joinSession(input, deps);
            if (problem) return { joinAddress: problem };
            const target = parseJoinLink(input);
            if (target) storage.setItem(LAST_KEY, formatTarget(target));
            return null;
        }
    });
}

/** The list of what was found. Show names come off the network: text only. */
export function foundRows(found: FoundSession[], join: (session: FoundSession) => void): ElementDefinition[] {
    return found.flatMap((session, index): ElementDefinition[] => {
        const row = `found${index}`;
        const info = `${row}info`;
        return [
            { type: "li", classes: "found-session", varId: row },
            { type: "div", classes: "found-info", varId: info, target: `@${row}` },
            { type: "span", classes: "found-name", text: session.show_name || "Unnamed show", target: `@${info}` },
            {
                type: "span", classes: "found-host", target: `@${info}`,
                text: session.compatible ? `${session.host}:${session.port}` : `${session.host} · other ShowFlow version, update to join`
            },
            {
                type: "button", classes: ["btn", "btn-primary"], text: "Join", target: `@${row}`,
                attributes: { type: "button", ...(session.compatible ? {} : { disabled: "disabled" }) },
                events: { click: () => join(session) }
            }
        ];
    });
}

export interface WatchDeps {
    find: () => Promise<FoundSession[]>;
    /** Called when what is on offer changed (and after the first search). */
    show: (found: FoundSession[]) => void;
    /** Called when searching itself fails, and with null once it works again. */
    problem: (message: string | null) => void;
    /** False once the list is gone: the search stops for good. */
    alive: () => boolean;
    /** True while nobody can see the list. */
    hidden: () => boolean;
    schedule?: (callback: () => void, ms: number) => unknown;
}

/** Searches again and again, one search at a time, while the list is on screen. */
export async function watchSessions(deps: WatchDeps): Promise<void> {
    const schedule = deps.schedule ?? ((callback, ms) => setTimeout(callback, ms));
    let shown: string | null = null;
    const round = async (): Promise<void> => {
        if (!deps.alive()) return;
        if (!deps.hidden()) {
            try {
                const found = await deps.find();
                if (!deps.alive()) return;
                deps.problem(null);
                // Redrawing an unchanged list would only take the focus and hover away.
                const next = JSON.stringify(found);
                if (next !== shown) {
                    shown = next;
                    deps.show(found);
                }
            } catch (error) {
                deps.problem(error instanceof Error ? error.message : String(error));
            }
        }
        schedule(() => { void round(); }, SEARCH_EVERY_MS);
    };
    await round();
}

export function renderDiscovery(): void {
    const { discovery } = getApi();
    if (!discovery) return;
    const parts = generateDynamic([
        { type: "div", id: "sessionDiscovery", varId: "root" },
        { type: "div", classes: "join-card", target: "@root", varId: "card" },
        { type: "h3", text: "Sessions on this network", target: "@card" },
        { type: "p", classes: "found-status", text: "Searching…", attributes: { role: "status" }, target: "@card", varId: "status" },
        { type: "ul", classes: "found-list", target: "@card", varId: "list" },
        { type: "p", classes: "join-error", attributes: { role: "alert" }, target: "@card", varId: "error" },
        { type: "button", classes: ["chip", "join-manual"], text: "Join by address…", attributes: { type: "button" }, events: { click: () => { void openManualJoin(); } }, target: "@card" }
    ], "#mainSection");

    const join = async (session: FoundSession): Promise<void> => {
        parts.error.textContent = "";
        const problem = await joinSession(`${session.host}:${session.port}`);
        if (problem) parts.error.textContent = problem;
    };

    void watchSessions({
        find: () => discovery.find(),
        show: found => {
            parts.status.textContent = found.length ? "" : "No sessions found yet. Devices need to be on the same network as the host.";
            parts.list.replaceChildren();
            const rows = foundRows(found, session => { void join(session); });
            if (rows.length) generateDynamic(rows, parts.list);
        },
        problem: message => {
            if (message) parts.status.textContent = `Could not search the network: ${message}`;
        },
        alive: () => parts.card.isConnected,
        hidden: () => document.hidden
    });
}
