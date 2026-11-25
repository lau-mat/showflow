import { generateDynamic } from "/shared/js/dynamic_generator";
import { loadPage } from "../page_loader";
import { renderSessionDiscovery } from "./home";

/** Mobile landing page: join a running session, or open the show manager. */
export function loadSessionsPage(): void {
    renderSessionDiscovery();
    generateDynamic([{
        type: "button", id: "openShowManagerBtn", text: "Show manager",
        events: { click: () => { void loadPage("home"); } }
    }], "#mainSection");
}
