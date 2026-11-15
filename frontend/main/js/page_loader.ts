import { loadHomePage } from "./pages/home";
import { createLoginPage } from "./pages/login";

const pages: { [key: string]: { load: () => void } } = {
    home: { load: loadHomePage },
    login: { load: createLoginPage }
};

function loadPage(pageName: string) {
    if (!pages[pageName]) throw new Error(`Page "${pageName}" not found`);
    pages[pageName].load();
    window.history.pushState({}, "", `#${pageName}`);
    window.location.hash = `#${pageName}`;
    document.title = `Showflow - ${pageName.charAt(0).toUpperCase() + pageName.slice(1)}`;
}

export { loadPage };