import { dynamicPrompt } from "/shared/js/dynamic_generator";

async function createLoginPage() {
    const loginData = await dynamicPrompt({
        title: "Login",
        elements: [
            { type: "input-label",label: "Username", id: "username" },
            { type: "input-label", label: "Password", inputType: "password", id: "password" }
        ],
        confirmText: "Login",
        required: true
    });

}

export { createLoginPage };