// The caller code rules, for instant feedback in forms. The server is the authority
// (showlogic/src/code.rs).

export const MIN_CODE_CHARS = 6;
export const MAX_CODE_CHARS = 32;

/** What is wrong with a code, or null when it is fine. Symbols are allowed, never required. */
export function codeProblem(code: string): string | null {
    if (!/^[\x21-\x7e]*$/.test(code)) return "Use letters, digits and symbols without spaces";
    if (code.length < MIN_CODE_CHARS) return `The code needs at least ${MIN_CODE_CHARS} characters`;
    if (code.length > MAX_CODE_CHARS) return `The code can be at most ${MAX_CODE_CHARS} characters`;
    return null;
}
