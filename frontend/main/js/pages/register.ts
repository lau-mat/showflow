import { dynamicPrompt } from "/shared/js/dynamic_generator";
import { getPlatform } from "/shared/js/platform";
import { getApi } from "../api";
import { ApiError } from "../api/types";
import { loadPage, takeReturnTo } from "../page_loader";
import { authFailure, MAX_PASSWORD_BYTES, MIN_PASSWORD_BYTES, pageLink, passwordBytes, validateEmail } from "./auth_form";

/** Creates an account and signs straight in with it. */
async function createRegisterPage() {
    const { session } = getApi();
    const profile = getPlatform();
    if (session.isAuthorized) return loadPage(takeReturnTo() ?? profile.landingPage);

    await dynamicPrompt({
        title: "Create account",
        elements: [
            { type: "input-label", label: "Username", id: "username" },
            { type: "input-label", label: "Email", id: "email" },
            { type: "input-label", label: "Password", inputType: "password", id: "password" },
            { type: "input-label", label: "Repeat password", inputType: "password", id: "passwordRepeat" },
            pageLink("I already have an account", profile.loginPage)
        ],
        confirmText: "Create account",
        required: true,
        isModal: false,
        onValidate: async (data): Promise<Record<string, string> | null> => {
            const username = String(data.username).trim();
            const email = String(data.email).trim();
            const password = String(data.password);

            if (!username) return { username: "Please choose a username" };
            if (!validateEmail(email)) return { email: "Invalid email format" };
            const bytes = passwordBytes(password);
            if (bytes < MIN_PASSWORD_BYTES) return { password: `Use at least ${MIN_PASSWORD_BYTES} characters` };
            if (bytes > MAX_PASSWORD_BYTES) return { password: "That password is too long" };
            if (password !== String(data.passwordRepeat)) return { passwordRepeat: "The passwords do not match" };

            try {
                await session.register(username, email, password);
            } catch (error) {
                // 409: the email or username is taken; anything else is about the form as a whole.
                return authFailure(error, error instanceof ApiError && error.status === 409 ? "email" : "username");
            }
            return null;
        }
    });

    await loadPage(takeReturnTo() ?? profile.landingPage);
}

export { createRegisterPage };
