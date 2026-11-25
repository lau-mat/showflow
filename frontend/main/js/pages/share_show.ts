import { addHoverBox, type ElementDefinition } from "/shared/js/dynamic_generator";
import { getPlatform } from "/shared/js/platform";
import { canManageShow, type MemberRole } from "/shared/js/show_types";
import { getApi } from "../api";
import { ApiError, NetworkError, type InviteInfo, type Invited, type Member } from "../api/types";

type Box = ReturnType<typeof addHoverBox>;

const ROLE_LABELS: Record<string, string> = { owner: "Owner", editor: "Can edit", commenter: "Can comment", viewer: "Can view" };
const GRANTABLE = ["editor", "commenter", "viewer"] as const;

interface ShareState {
    members: Member[];
    invites: InviteInfo[];
    /** What the last "Invite" produced, shown until the next action. */
    result: Invited | null;
    notice: string | null;
}

const env = (import.meta as { env?: Record<string, string | undefined> }).env;

/**
 * The address people open to accept an invite. The web knows its own; the apps are not served
 * from a public address, so they need `VITE_PUBLIC_URL` (otherwise the bare invite code is shown).
 */
export function inviteLink(token: string): string {
    const base = env?.VITE_PUBLIC_URL ?? (getPlatform().kind === "web" ? `${location.origin}${location.pathname}` : null);
    return base ? `${base.replace(/#.*$/, "")}#invite/${token}` : token;
}

function describeInvite(invite: InviteInfo): string {
    const who = invite.email ?? "anyone with the link";
    return `${ROLE_LABELS[invite.role] ?? invite.role} · ${who} · ${invite.used_count}/${invite.max_uses} used · until ${new Date(invite.expires_at).toLocaleDateString()}`;
}

function shareContent(showId: string, myRole: MemberRole, state: ShareState, box: () => Box): ElementDefinition[] {
    const { share } = getApi();
    const owner = canManageShow(myRole);
    const refresh = async (patch: Partial<ShareState> = {}) => render(showId, myRole, box(), patch);
    const attempt = (action: () => Promise<unknown>, patch: Partial<ShareState> = {}) => async () => {
        try {
            await action();
            await refresh(patch);
        } catch (error) {
            await refresh({ notice: error instanceof NetworkError ? "You need to be online to change who has access." : error instanceof Error ? error.message : String(error) });
        }
    };

    const memberRows: ElementDefinition[] = state.members.flatMap((member): ElementDefinition[] => {
        const row = `member_${member.user_id}`;
        const canChange = owner && !member.is_me && member.role !== "owner";
        return [
            { type: "li", classes: "device-row", varId: row, target: "@memberList" },
            { type: "span", classes: "device-name", text: member.is_me ? `${member.username} (you)` : member.username, target: `@${row}` },
            canChange
                ? { type: "select", varId: `${row}_role`, classes: "role-select", target: `@${row}`,
                    events: { change: (event: Event) => { void attempt(() => share.setRole(showId, member.user_id, (event.target as HTMLSelectElement).value))(); } } }
                : { type: "span", classes: "device-seen", text: ROLE_LABELS[member.role] ?? member.role, target: `@${row}` },
            ...(canChange ? GRANTABLE.map(role => ({
                type: "option", text: ROLE_LABELS[role], attributes: { value: role, ...(member.role === role ? { selected: "selected" } : {}) }, target: `@${row}_role`
            })) : []),
            ...((canChange || (member.is_me && member.role !== "owner")) ? [{
                type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"],
                attributes: { title: member.is_me ? "Leave this show" : "Remove from this show" },
                events: { click: attempt(() => share.removeMember(showId, member.user_id)) },
                target: `@${row}`
            }] : [])
        ];
    });

    const inviteRows: ElementDefinition[] = state.invites.flatMap((invite): ElementDefinition[] => {
        const row = `invite_${invite.id}`;
        return [
            { type: "li", classes: "device-row", varId: row, target: "@inviteList" },
            { type: "span", classes: "device-name", text: describeInvite(invite), target: `@${row}` },
            {
                type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], attributes: { title: "Revoke this invitation" },
                events: { click: attempt(() => share.revokeInvite(showId, invite.id)) }, target: `@${row}`
            }
        ];
    });

    const result: ElementDefinition[] = [];
    if (state.result?.added) {
        result.push({ type: "p", classes: "account-email", text: `${state.result.added.username} now has access.`, target: "@shareBody" });
    } else if (state.result?.token) {
        const link = inviteLink(state.result.token);
        result.push(
            { type: "p", classes: "account-email", text: link === state.result.token ? "No account with that email yet. Send them this invite code (they enter it after #invite/):" : "No account with that email yet. Send them this link:", target: "@shareBody" },
            { type: "div", classes: "invite-link-row", varId: "linkRow", target: "@shareBody" },
            { type: "input", id: "inviteLink", attributes: { type: "text", readonly: "readonly", value: link }, target: "@linkRow" },
            {
                type: "button", classes: ["btn", "btn-primary"], text: "Copy", target: "@linkRow",
                events: { click: () => { void navigator.clipboard?.writeText(link); } }
            }
        );
    }

    const inviteForm: ElementDefinition[] = owner ? [
        { type: "h3", text: "Invite someone", target: "@shareBody" },
        { type: "div", classes: "invite-link-row", varId: "inviteRow", target: "@shareBody" },
        { type: "select", id: "inviteRole", classes: "role-select", varId: "inviteRoleSelect", target: "@inviteRow" },
        ...GRANTABLE.map(role => ({ type: "option", text: ROLE_LABELS[role], attributes: { value: role }, target: "@inviteRoleSelect" })),
        { type: "input", id: "inviteEmail", attributes: { type: "text", placeholder: "Email (optional)" }, target: "@inviteRow" },
        {
            type: "button", classes: ["btn", "btn-primary"], text: "Invite", target: "@inviteRow",
            events: {
                click: async () => {
                    const role = (document.getElementById("inviteRole") as HTMLSelectElement).value as "editor" | "commenter" | "viewer";
                    const email = (document.getElementById("inviteEmail") as HTMLInputElement).value.trim();
                    try {
                        const result = await share.createInvite(showId, { role, ...(email ? { email } : {}) });
                        await refresh({ result, notice: null });
                    } catch (error) {
                        await refresh({ notice: error instanceof ApiError ? error.message : "Could not create the invitation." });
                    }
                }
            }
        },
        ...result,
        ...(state.invites.length > 0 ? [
            { type: "h3", text: "Open invitations", target: "@shareBody" },
            { type: "ul", classes: "device-list", varId: "inviteList", target: "@shareBody" },
            ...inviteRows
        ] : [])
    ] : [];

    return [
        { type: "div", classes: "account-body", varId: "shareBody" },
        ...(state.notice ? [{ type: "p", classes: "modal-error-message", text: state.notice, target: "@shareBody" }] : []),
        { type: "h3", text: "People with access", target: "@shareBody" },
        { type: "ul", classes: "device-list", varId: "memberList", target: "@shareBody" },
        ...memberRows,
        ...inviteForm
    ];
}

async function render(showId: string, myRole: MemberRole, box: Box, patch: Partial<ShareState> = {}): Promise<void> {
    const { share } = getApi();
    const previous = (box as unknown as { shareState?: ShareState }).shareState;
    const [members, invites] = await Promise.all([
        share.members(showId),
        canManageShow(myRole) ? share.invites(showId) : Promise.resolve([])
    ]);
    const state: ShareState = { members, invites, result: previous?.result ?? null, notice: null, ...patch };
    (box as unknown as { shareState?: ShareState }).shareState = state;
    box.refreshContent(shareContent(showId, myRole, state, () => box));
    // Leaving removes your own access: nothing left to show.
    if (!members.some(member => member.is_me)) box.close();
}

export async function openShareBox(showId: string, showName: string, myRole: MemberRole): Promise<void> {
    let box: Box;
    box = addHoverBox(null, `Share — ${showName}`, [], { width: "560px", height: "auto" });
    try {
        await render(showId, myRole, box);
    } catch (error) {
        const message = error instanceof NetworkError ? "Sharing needs an internet connection." : error instanceof Error ? error.message : String(error);
        box.refreshContent([{ type: "p", classes: "modal-error-message", text: message }]);
    }
}
