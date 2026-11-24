import { dynamicPrompt } from "/shared/js/dynamic_generator";
import { api } from "../api";

function validateEmail(email: string): boolean {
  return !!email
    .toLowerCase()
    .match(
      /^(([^<>()[\]\\.,;:\s@"]+(\.[^<>()[\]\\.,;:\s@"]+)*)|.(".+"))@((\[[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\])|(([a-zA-Z\-0-9]+\.)+[a-zA-Z]{2,}))$/
    );
}

async function createLoginPage() {
    const loginData = await dynamicPrompt({
        title: "Login",
        elements: [
            { type: "input-label", label: "Email", id: "email" },
            { type: "input-label", label: "Password", inputType: "password", id: "password" }
        ],
        confirmText: "Login",
        required: true,
        isModal: false,
        onValidate: async data =>  {
            const { email, password } = data;

            if(!validateEmail(String(email))) return {email: "Invalid email format"};

            const response = await api.getResponse("auth/login", { email, password });
            if(response.error) return {email: response.error};

            return {email: "wrong email"};
        }
    });

    console.log(loginData);
}

export { createLoginPage };