import type { PageRegistry } from "../page_loader";
import { loadHomePage } from "./home";
import { loadAcceptInvitePage } from "./accept_invite";
import { createLoginPage } from "./login";
import { createRegisterPage } from "./register";

export const pages: PageRegistry = {
    login: { load: createLoginPage, access: "public" },
    register: { load: createRegisterPage, access: "public" },
    home: { load: loadHomePage, access: "auth", feature: "showEditor" },
    invite: { load: loadAcceptInvitePage, access: "auth" }
};
