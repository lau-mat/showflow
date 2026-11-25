// Lets node:test import the frontend TypeScript directly (Node strips the types):
// resolves the Vite "/shared" alias and swaps Tauri / DOM-bound modules for stubs.
import { registerHooks } from "node:module";

const frontend = new URL("../../frontend/", import.meta.url);
const stubs = {
    "@tauri-apps/api/core": "export const invoke = (...args) => globalThis.__invoke(...args);",
    "@tauri-apps/api/event": "export const listen = (...args) => globalThis.__listen(...args);",
    "@tauri-apps/api/window": "export const getCurrentWindow = () => globalThis.__currentWindow;",
    "/shared/js/dynamic_generator":
        "export const registerCompound = () => {};"
        + "export const addHoverBox = () => ({ container: { style: {} }, refreshContent() {} });"
        + "export const dynamicGenerator = (...args) => globalThis.__dynamicGenerator(...args);"
        + "export const generateDynamic = (...args) => globalThis.__generateDynamic(...args);"
        + "export const dynamicPrompt = (...args) => globalThis.__dynamicPrompt(...args);"
};

globalThis.__dynamicGenerator = () => { throw new Error("dynamicGenerator is stubbed"); };
globalThis.__listen = async () => () => {};
globalThis.__generateDynamic = () => { throw new Error("generateDynamic is stubbed"); };
globalThis.__dynamicPrompt = async () => null;

registerHooks({
    resolve(specifier, context, next) {
        if (specifier in stubs) {
            return { url: `data:text/javascript,${encodeURIComponent(stubs[specifier])}`, shortCircuit: true };
        }
        if (specifier.startsWith("/shared/")) {
            const path = /\.\w+$/.test(specifier) ? specifier : `${specifier}.ts`;
            return { url: new URL(`.${path}`, frontend).href, shortCircuit: true };
        }
        // Vite resolves extensionless relative imports; Node does not.
        if (specifier.startsWith(".") && !/\.\w+$/.test(specifier)) {
            try {
                return next(`${specifier}.ts`, context);
            } catch {
                return next(`${specifier}/index.ts`, context);
            }
        }
        return next(specifier, context);
    }
});
