import type { ElementDefinition } from "/shared/js/dynamic_generator";
import { ApiError, NetworkError } from "../api/types";

/** Mirrors the server: passwords are 12 to 1024 bytes (backend/src/auth.rs `register`). */
export const MIN_PASSWORD_BYTES = 12;
export const MAX_PASSWORD_BYTES = 1024;

export function validateEmail(email: string): boolean {
    return !!email
        .toLowerCase()
        .match(
            /^(([^<>()[\]\\.,;:\s@"]+(\.[^<>()[\]\\.,;:\s@"]+)*)|.(".+"))@((\[[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\])|(([a-zA-Z\-0-9]+\.)+[a-zA-Z]{2,}))$/
        );
}

export function passwordBytes(password: string): number {
    return new TextEncoder().encode(password).length;
}

/**
 * Turns a failed auth request into a field error for the form, or rethrows what is not
 * the user's doing (bugs). `field` is where server-side rejections are shown.
 */
export function authFailure(error: unknown, field: string): Record<string, string> {
    if (error instanceof NetworkError) return { [field]: "Cannot reach the ShowFlow server. Check your connection." };
    if (error instanceof ApiError) return { [field]: error.message };
    throw error;
}

/** A plain link to another public page; the hash router does the navigation. */
export function pageLink(text: string, page: string): ElementDefinition {
    return { type: "a", classes: "auth-link", text, attributes: { href: `#${page}` } };
}
