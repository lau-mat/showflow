import type { Feature, PlatformProfile } from "/shared/js/platform";

export interface PageDef {
    /** `params` is whatever follows the page name in the hash (`#invite/<token>` gives `<token>`). */
    load: (params: string) => void | Promise<void>;
    /** "auth" pages need a signed-in (or offline-grace) user. */
    access: "public" | "auth";
    /** Page is only reachable where this feature exists. */
    feature?: Feature;
}

export type PageRegistry = Record<string, PageDef>;

/** Decides which page a navigation request actually lands on. */
export function resolvePage(
    requested: string,
    pages: PageRegistry,
    profile: PlatformProfile,
    authorized: boolean
): string {
    const page = pages[requested];
    if (!page) return authorized ? profile.landingPage : profile.loginPage;
    if (page.access === "auth" && !authorized) return profile.loginPage;
    if (page.feature && !profile.features[page.feature]) {
        return authorized ? profile.landingPage : profile.loginPage;
    }
    return requested;
}

export interface RouterDeps {
    pages: PageRegistry;
    profile: PlatformProfile;
    isAuthorized: () => boolean;
    /** Defaults to the real window/document; injectable for tests. */
    win?: Pick<Window, "location" | "addEventListener">;
    doc?: Pick<Document, "getElementById"> & Partial<Pick<Document, "querySelectorAll">> & { title: string };
}

/** The url hash is the single source of truth for the current page. */
export class Router {
    private readonly deps: RouterDeps;
    private readonly win: NonNullable<RouterDeps["win"]>;
    private readonly doc: NonNullable<RouterDeps["doc"]>;

    constructor(deps: RouterDeps) {
        this.deps = deps;
        this.win = deps.win ?? window;
        this.doc = deps.doc ?? document;
    }

    private get hash(): string {
        return this.win.location.hash.replace(/^#/, "");
    }

    /** Routes the initial hash and follows later changes. */
    start(): Promise<void> {
        this.win.addEventListener("hashchange", () => { this.navigate(this.hash).catch(console.error); });
        return this.navigate(this.hash);
    }

    async navigate(requested: string): Promise<void> {
        const { pages, profile, isAuthorized } = this.deps;
        const [name, ...rest] = requested.split("/");
        const params = rest.join("/");
        const target = resolvePage(name, pages, profile, isAuthorized());
        // A page that needs a sign-in: remember where the user was headed (an invite link, say),
        // so signing in or registering leads there instead of to the landing page.
        if (target !== name && pages[name]?.access === "auth" && !isAuthorized()) rememberReturnTo(requested);
        const wanted = target === name ? requested : target;
        if (this.hash !== wanted) {
            // The resulting hashchange renders the page.
            this.win.location.hash = `#${wanted}`;
            return;
        }
        this.doc.getElementById("mainSection")?.replaceChildren();
        // Prompts (login, register) live on <body>, not in #mainSection, so a page that is
        // navigated away from while its prompt is open (the login <-> register links) would
        // leave that prompt on top of the next page.
        this.doc.querySelectorAll?.(".dialog-box.modal-box").forEach(dialog => dialog.remove());
        this.doc.title = `Showflow - ${target.charAt(0).toUpperCase()}${target.slice(1)}`;
        await pages[target].load(params);
    }
}

const RETURN_KEY = "showflow.return_to";

function sessionStore(): Pick<Storage, "getItem" | "setItem" | "removeItem"> | null {
    try {
        return globalThis.sessionStorage ?? null;
    } catch {
        return null;
    }
}

function rememberReturnTo(hash: string): void {
    sessionStore()?.setItem(RETURN_KEY, hash);
}

/** Where to go after signing in: the page the user originally asked for, once. */
export function takeReturnTo(): string | null {
    const store = sessionStore();
    const hash = store?.getItem(RETURN_KEY) ?? null;
    store?.removeItem(RETURN_KEY);
    return hash;
}

let router: Router | null = null;

export function initRouter(deps: RouterDeps): Router {
    router = new Router(deps);
    return router;
}

export function loadPage(pageName: string): Promise<void> {
    if (!router) throw new Error("Router not initialised yet.");
    return router.navigate(pageName);
}
