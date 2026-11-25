import { dynamicPrompt, type ElementDefinition } from "/shared/js/dynamic_generator";
import { getPlatform } from "/shared/js/platform";
import { getApi } from "../api";
import { ApiError, NetworkError } from "../api/types";
import { loadPage } from "../page_loader";

const ROLE_TEXT: Record<string, string> = { editor: "edit", commenter: "comment on", viewer: "view" };

/** `#invite/<token>`: shows what the invitation is for and joins on confirmation. */
export async function loadAcceptInvitePage(token: string): Promise<void> {
    const { share, sync } = getApi();
    const landing = getPlatform().landingPage;

    let message: ElementDefinition[];
    try {
        const preview = await share.previewInvite(token);
        if (preview.already_member) return void (await loadPage(landing));
        const by = preview.invited_by ? `${preview.invited_by} invited you to ` : "You are invited to ";
        message = [{ type: "p", text: `${by}${ROLE_TEXT[preview.role] ?? preview.role} "${preview.show_name}".` }];
    } catch (error) {
        const text = error instanceof NetworkError
            ? "Cannot reach the ShowFlow server. Check your connection and open the link again."
            : error instanceof ApiError ? error.message : "This invitation could not be opened.";
        await dynamicPrompt({ title: "Invitation", elements: [{ type: "p", classes: "modal-error-message", text }], confirmText: "Continue", required: true, isModal: false });
        return void (await loadPage(landing));
    }

    const answer = await dynamicPrompt({
        title: "Invitation",
        elements: message,
        confirmText: "Join",
        cancelText: "Not now",
        isModal: false,
        onValidate: async (): Promise<Record<string, string> | null> => {
            try {
                await share.acceptInvite(token);
                return null;
            } catch (error) {
                return { _: error instanceof ApiError ? error.message : "Could not join. Try again." };
            }
        }
    });
    // Apps pull the newly shared show into their local database right away.
    if (answer) sync?.trigger(0);
    await loadPage(landing);
}
