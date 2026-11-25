import { dynamicPrompt, type ElementDefinition } from "/shared/js/dynamic_generator";
import { getPlatform } from "/shared/js/platform";
import { getApi } from "../api";
import { loadPage, takeReturnTo } from "../page_loader";
import { authFailure, pageLink, validateEmail } from "./auth_form";

async function createLoginPage() {
    const { session } = getApi();
    const profile = getPlatform();
    if (session.isAuthorized) return loadPage(takeReturnTo() ?? profile.landingPage);

    const notice: ElementDefinition[] = session.state === "graceExpired"
        ? [{ type: "p", classes: "modal-error-message", text: "The offline period has ended. Connect to the internet and log in again." }]
        : [];

    await dynamicPrompt({
        title: "Login",
        elements: [
            ...notice,
            { type: "input-label", label: "Email", id: "email" },
            { type: "input-label", label: "Password", inputType: "password", id: "password" },
            pageLink("Create an account", profile.registerPage)
        ],
        confirmText: "Login",
        required: true,
        isModal: false,
        onValidate: async (data): Promise<Record<string, string> | null> => {
            const email = String(data.email);
            const password = String(data.password);

            if (!validateEmail(email)) return { email: "Invalid email format" };
            if (!password) return { password: "Please fill in your password" };

            try {
                await session.login(email, password);
            } catch (error) {
                return authFailure(error, "password");
            }
            return null;
        }
    });

    await loadPage(takeReturnTo() ?? profile.landingPage);
}

export { createLoginPage };
