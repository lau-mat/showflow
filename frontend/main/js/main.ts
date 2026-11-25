import { detectPlatform } from "/shared/js/platform";
import { initApi } from "./api";
import { initRouter } from "./page_loader";
import { mountAccountMenu } from "./pages/account";
import { pages } from "./pages/registry";

async function bootstrap(): Promise<void> {
    const profile = await detectPlatform();
    const { session, sync, syncStatus, live } = await initApi(profile);
    await session.restore();

    // Coming back online while inside the offline grace window re-validates the session.
    window.addEventListener("online", () => { void session.retryOnline(); sync?.trigger(0); });
    window.addEventListener("focus", () => sync?.trigger(0));
    sync?.start();
    live.start();

    mountAccountMenu(session, sync ? syncStatus : null);
    await initRouter({ pages, profile, isAuthorized: () => session.isAuthorized }).start();
}

bootstrap().catch(console.error);
