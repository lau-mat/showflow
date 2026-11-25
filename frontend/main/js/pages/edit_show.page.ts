import { addHoverBox, dynamicGenerator, dynamicPrompt, generateDynamic, type ElementDefinition } from "/shared/js/dynamic_generator";
import "/shared/js/compound_components";
import { hasFeature } from "/shared/js/platform";
import { formatScenarioLineDuration, formatScenarioLineTime, resolveScenarioLineStartTimestamp } from "/shared/js/scenario_time";
import { composeDateTime, decomposeDateTime } from "/shared/js/time_math";
import { getApi } from "../api";
import type { LineEdit, ShowEdit } from "../api/types";
import { openSessionSettings } from "./session_settings";
import { openShareBox } from "./share_show";
import { canComment, canEditContent, sortCommentsByRole, type ScenarioLine, type ScenarioLineComment, type ShowDetails, type ShowRole } from "/shared/js/show_types";

type PromptData = NonNullable<Awaited<ReturnType<typeof dynamicPrompt>>>;

let currentBox: ReturnType<typeof addHoverBox> | null = null;
let currentShowId: string | null = null;
let currentShowData: ShowDetails | null = null;
const hiddenRoleIds = new Set<string>();

function requireShowData(): ShowDetails {
    if (!currentShowData) throw new Error("No show loaded in the editor.");
    return currentShowData;
}

function requireShowId(): string {
    if (currentShowId === null) throw new Error("No show selected in the editor.");
    return currentShowId;
}

let stopWatching: (() => void) | null = null;

async function openEditShow(id: string): Promise<void> {
    currentShowId = id;
    // Re-render when another device or person changes this show (sync, or the live stream).
    stopWatching?.();
    stopWatching = getApi().data.subscribe(change => {
        const open = currentBox !== null && currentBox.container.style.display !== "none";
        if (!open || (change.showId !== null && change.showId !== currentShowId)) return;
        // Our own edit already refreshed the sheet; the announcement of it is not news.
        if (change.rev !== undefined && currentShowData && change.rev <= currentShowData.rev) return;
        refreshShowData().catch(() => {
            // The show disappeared (deleted, or access removed): there is nothing left to edit.
            currentBox?.close();
            stopWatching?.();
            stopWatching = null;
        });
    });
    await refreshShowData();
}

async function refreshShowData(): Promise<void> {
    if (currentShowId === null) throw new Error("No show selected in the editor.");
    const data = await getApi().data.getShowDetails(currentShowId);
    currentShowData = data;

    // If the box is already open, just refresh its content dynamically
    if (currentBox && currentBox.container.style.display !== "none") {
        const title = currentBox.header?.querySelector("h2");
        if (title) title.textContent = `Show Manager — ${data.show.name}`;
        currentBox.refreshContent(buildEditShowLayout(data));
    } else {
        currentBox = addHoverBox(
            null,
            `Show Manager — ${data.show.name}`,
            buildEditShowLayout(data),
            { fullscreen: true }
        );
    }
}

/**
 * Builds the dynamic definition array for the full screen editor layout
 */
function buildEditShowLayout(data: ShowDetails): ElementDefinition[] {
    // What this member may do; the server enforces the same rules, this just hides what would be refused.
    const role = data.show.role ?? "owner";
    const mayEdit = canEditContent(role);
    const content: ElementDefinition[] = [
        // Base Layout Containers
        { type: "div", classes: "show-editor-container", varId: "editorContainer" },
        { type: "aside", classes: "show-editor-sidebar", varId: "sidebar", target: "@editorContainer" },

        // Sidebar: Details Card
        { type: "div", classes: "editor-card", varId: "editorCard", target: "@sidebar" },
        { type: "h3", text: "Show Details", target: "@editorCard" },
        { type: "div", classes: "stat-grid", varId: "stats", target: "@editorCard" },
        ...showStats(data).flatMap(([label, value]) => [
            { type: "span", classes: "stat-label", text: label, data: { stat: label.toLowerCase() }, target: "@stats" },
            { type: "span", classes: "stat-value", text: value, data: { stat: label.toLowerCase() }, target: "@stats" }
        ]),

        // Sidebar: Roles Card
        { type: "div", classes: "editor-card", varId: "roleCard", target: "@sidebar" },
        { type: "div", classes: "card-header-flex", varId: "roleCardHeader", target: "@roleCard" },
        { type: "h3", text: "Roles", target: "@roleCardHeader" },
        ...(mayEdit ? [{ type: "svg", classes: ["btn-icon", "btn-outline-primary"], file: "icons/add.svg", events: { click: addRole }, target: "@roleCardHeader" }] : []),
        { type: "ul", classes: "role-list", varId: "roleCardList", target: "@roleCard" },

        // Main Area: Run Sheet
        { type: "section", classes: "show-editor-main", varId: "main", target: "@editorContainer" },
        { type: "div", classes: "card-header-flex", varId: "mainHeader", target: "@main" },
        { type: "h3", text: "Run Sheet", target: "@mainHeader" },

        // Dedicated flex wrapper for header buttons
        { type: "div", classes: "header-actions-group", varId: "mainHeaderActions", target: "@mainHeader" },
        ...(mayEdit ? [
            { type: "svg", classes: ["btn-icon", "btn-outline-primary"], file: "icons/add.svg", attributes: { title: "Add cue" }, events: { click: addLine }, target: "@mainHeaderActions" },
            { type: "svg", classes: ["btn-icon", "btn-muted"], file: "icons/edit.svg", attributes: { title: "Edit show details" }, events: { click: editShowDetails }, target: "@mainHeaderActions" }
        ] : []),
        // Who may join and call this show's sessions: a setting of this computer, whatever the role.
        ...(hasFeature("sessionHost")
            ? [{ type: "svg", classes: ["btn-icon", "btn-muted"], file: "icons/settings.svg", attributes: { title: "Session settings" }, events: { click: () => { void openSessionSettings(data.show); } }, target: "@mainHeaderActions" }]
            : []),
        { type: "svg", classes: ["btn-icon", "btn-muted"], file: "icons/share.svg", attributes: { title: "Share" }, events: { click: () => openShareBox(data.show.id, data.show.name, role) }, target: "@mainHeaderActions" },
        ...(hasFeature("print")
            ? [{ type: "svg", classes: ["btn-icon", "btn-muted"], file: "icons/print.svg", events: { click: openPrintDialog }, target: "@mainHeaderActions" }]
            : []),

        // Table Architecture
        { type: "div", classes: "lines-table-wrapper", varId: "tableWrapper", target: "@main" },
        { type: "table", classes: "lines-table", varId: "table", target: "@tableWrapper" },
        { type: "thead", varId: "tableHeader", target: "@table" },
        { type: "tr", varId: "tableHeaderRow", target: "@tableHeader" },
        { type: "th", text: "Time", style: { width: "76px" }, target: "@tableHeaderRow" },
        { type: "th", text: "Cue", style: { width: "20%" }, target: "@tableHeaderRow" },
        { type: "th", text: "Note", target: "@tableHeaderRow" },
        { type: "th", text: "Comments", style: { width: "34%" }, target: "@tableHeaderRow" },
        { type: "th", style: { width: "92px" }, target: "@tableHeaderRow" },
        { type: "tbody", varId: "tableBody", target: "@table" }
    ];

    // Dynamic Roles Population (Spreading flat array)
    const roleItems: ElementDefinition[] = data.roles.length > 0
        ? data.roles.flatMap(role => [
            { type: "li", classes: "role-badge", varId: `roleLi_${role.id}`, target: "@roleCardList" },
            { type: "span", classes: "role-badge-label", text: role.name, target: `@roleLi_${role.id}` },
            {
                type: "svg",
                file: "icons/visible.svg",
                classes: [
                    "btn-icon",
                    "btn-role-toggle",
                    hiddenRoleIds.has(role.id) ? "is-hidden" : ""
                ].filter(Boolean),
                events: {
                    click: (e: Event) => {
                        e.stopPropagation();
                        toggleRoleVisibility(role.id, e.currentTarget as Element);
                    }
                },
                target: `@roleLi_${role.id}`
            }
        ])
        : [{ type: "small", classes: "empty-msg", text: "No roles created yet.", target: "@roleCardList" }];

    // Dynamic Scenario Lines Population (Spreading flat array)
    const lineItems: ElementDefinition[] = data.lines.length > 0
        ? data.lines.flatMap((line, index) => buildLineRow(
            line, data.comments, data.roles,
            resolveScenarioLineStartTimestamp(data.show.time, data.lines, index),
            role
        ))
        : [{
            type: "tr",
            target: "@tableBody",
            children: [{
                type: "td",
                attributes: { colspan: "5" },
                classes: "empty-table-cell",
                text: mayEdit
                ? "No scenario lines added yet. Click '+' above to start building your run sheet."
                : "Nothing in this run sheet yet."
            }]
        }];

    // Flatten everything into a single level array for dynamicGenerator
    return [...content, ...roleItems, ...lineItems];
}

/**
 * Label/value pairs for the sidebar: start, end, total duration and counts
 */
function showStats(data: ShowDetails): [string, string][] {
    const total = data.lines.reduce((sum, line) => sum + line.duration, 0);
    const last = data.lines.length - 1;
    const lastStart = last >= 0 ? resolveScenarioLineStartTimestamp(data.show.time, data.lines, last) : null;
    const end = lastStart === null ? null : lastStart + data.lines[last].duration;
    return [
        ["Date", new Date(data.show.time * 1000).toLocaleDateString()],
        ["Start", formatScenarioLineTime(data.show.time)],
        ["End", end === null ? "—" : formatScenarioLineTime(end)],
        ["Length", formatScenarioLineDuration(total)],
        ["Cues", String(data.lines.length)],
        ["Notes", String(data.comments.length)]
    ];
}

/**
 * On a phone a cue is a card that reveals its actions when tapped (the CSS only reacts to
 * `is-open` in the compact layout); taps on buttons and comments keep their own meaning.
 */
function toggleCue(event: Event): void {
    if ((event.target as Element).closest(".btn-icon, .comment-bubble")) return;
    (event.currentTarget as HTMLElement).classList.toggle("is-open");
}

/**
 * Builds a single table row for a Scenario Line and renders attached Comments inline
 */
function buildLineRow(
    line: ScenarioLine,
    allComments: ScenarioLineComment[],
    allRoles: ShowRole[],
    startTimestamp: number | null,
    role: ShowDetails["show"]["role"] = "owner"
): ElementDefinition[] {
    const mayEdit = canEditContent(role);
    const mayComment = canComment(role);
    const lineComments = allComments.filter(c => c.line_id === line.id);

    const content: ElementDefinition[] = [
        { type: "tr", target: "@tableBody", data: { id: line.id }, varId: "lineRow", events: { click: toggleCue } },
        { type: "td", classes: "line-number-cell", children: [
            { type: "div", classes: "startTime", text: formatScenarioLineTime(startTimestamp) },
            { type: "div", classes: "duration", text: formatScenarioLineDuration(line.duration) }
        ], target: "@lineRow" },
        { type: "td", classes: "line-content-cell", text: line.name, target: "@lineRow" },
        { type: "td", classes: "line-note-cell", text: line.note, target: "@lineRow" },
        { type: "td", classes: "line-comments-cell", varId: "commentsCell", target: "@lineRow" },
        { type: "td", classes: "line-action-cell", varId: "actionCell", target: "@lineRow" },
        { type: "div", classes: "line-action-cell-wrapper", varId: "actionWrapper", target: "@actionCell" },
        ...(mayComment ? [{ type: "svg", file: "icons/add.svg", classes: ["btn-icon", "btn-outline-primary"], attributes: { title: "Add note" }, events: { click: () => addComment(line.id) }, target: "@actionWrapper" }] : []),
        ...(mayEdit ? [
            { type: "svg", file: "icons/edit.svg", classes: ["btn-icon", "btn-muted"], attributes: { title: "Edit cue" }, events: { click: () => editLine(line.id) }, target: "@actionWrapper" },
            { type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], events: { click: () => deleteLine(line.id) }, target: "@actionWrapper" }
        ] : [])
    ];

    const commentEvents = (comment: ScenarioLineComment): Record<string, EventListener> =>
        mayComment ? { click: () => { void addComment(comment.line_id, comment.id); } } : {};
    const notes: ElementDefinition[] = sortCommentsByRole(lineComments, allRoles).map(comment => {
        const assignedRole = allRoles.find(r => r.id === comment.role_id);
        return {
            type: "div",
            classes: "comment-bubble",
            target: "@commentsCell",
            children: [
                ...(assignedRole ? [{ type: "span", classes: "comment-role-tag", text: assignedRole.name }] : []),
                { type: "span", text: comment.comment, classes: "comment-body-text" }
            ],
            data: { commentRoleId: String(comment.role_id) },
            style: { cursor: mayComment ? "pointer" : "default" },
            events: commentEvents(comment)
        };
    });

    return [...content, ...notes];
}

async function addRole(): Promise<void> {
    const newRoleData = await dynamicPrompt({
        title: "New Role",
        confirmText: "Create",
        elements: [
            { type: "input-label", id: "newRoleName", label: "Role name" }
        ],
        onValidate: data => {
            const errors: Record<string, string> = {};
            if (!data.newRoleName) errors.newRoleName = "Please fill in a role name";
            return errors;
        }
    });

    if (!newRoleData || !newRoleData.newRoleName) return;

    await getApi().data.addRole(requireShowId(), String(newRoleData.newRoleName));
    await refreshShowData();
}

interface LineForm {
    name: string;
    note: string;
    duration: number;
    absoluteTime: number | null;
}

/** The dialog for a cue, empty for a new one or filled in from `line` when editing. */
async function promptLine(showData: ShowDetails, options: { title: string; confirmText: string; line?: ScenarioLine }): Promise<LineForm | null> {
    const { line } = options;
    // A fixed start time is a time of day on the show's date; offer the show's own start as the default.
    const showStartOfDay = decomposeDateTime(showData.show.time).secondsOfDay;
    const fixed = line?.absolute_time != null;

    const lineData = await dynamicPrompt({
        title: options.title,
        confirmText: options.confirmText,
        elements: [
            { type: "input-label", id: "lineName", label: "Cue name", placeholder: "e.g., Intro Cue", value: line?.name },
            { type: "time-label", id: "duration", label: "Duration", config: { mode: "duration" }, value: line?.duration ?? 60 },
            {
                type: "choice-label", id: "startMode", label: "Starts", value: fixed ? "absolute" : "follow",
                config: { options: [{ value: "follow", label: "After the previous cue" }, { value: "absolute", label: "At a fixed time" }] },
                events: {
                    change: (event: Event) => {
                        const group = document.getElementById("startTime-group");
                        if (group) group.style.display = (event.target as HTMLInputElement).value === "absolute" ? "" : "none";
                    }
                }
            },
            {
                type: "time-label", id: "startTime", label: "Start time", config: { mode: "clock" },
                value: fixed ? decomposeDateTime(line!.absolute_time!).secondsOfDay : showStartOfDay,
                style: fixed ? {} : { display: "none" }
            },
            { type: "textarea-label", id: "lineContent", label: "Cue description", placeholder: "Enter cue description...", rows: 3, value: line?.note.trim() }
        ],
        onValidate: data => {
            const errors: Record<string, string> = {};
            const lineName = String(data.lineName);
            if (lineName.length < 5 || lineName.length > 100)
                errors.lineName = "Cue Name must be between 5 and 100 characters.";
            if (Number(data.duration) === 0) errors.duration = "Select the duration of this item";
            return errors;
        }
    });
    if (!lineData) return null;

    let absoluteTime: number | null = null;
    if (lineData.startMode === "absolute") {
        const { date } = decomposeDateTime(showData.show.time);
        absoluteTime = composeDateTime(date, Number(lineData.startTime));
    }
    return {
        name: String(lineData.lineName),
        note: String(lineData.lineContent ?? "").trim(),
        duration: Number(lineData.duration),
        absoluteTime
    };
}

/** Name and start time of the show; saves only what changed so a simultaneous edit by someone else survives. */
async function editShowDetails(): Promise<void> {
    const showData = requireShowData();
    const { show } = showData;
    const form = await dynamicPrompt({
        title: "Edit Show",
        confirmText: "Save",
        elements: [
            { type: "input-label", id: "showName", label: "Show name", value: show.name },
            { type: "datetime-label", id: "showTime", label: "Show time", value: show.time }
        ],
        onValidate: data => {
            const errors: Record<string, string> = {};
            if (!String(data.showName).trim()) errors.showName = "Please fill in a name";
            if (!Number(data.showTime)) errors.showTime = "A start time must be selected";
            return errors;
        }
    });
    if (!form) return;

    const changes: ShowEdit = {};
    const name = String(form.showName).trim();
    if (name !== show.name) changes.name = name;
    if (Number(form.showTime) !== show.time) changes.time = Number(form.showTime);
    if (Object.keys(changes).length === 0) return;

    await getApi().data.editShow(requireShowId(), changes);
    await refreshShowData();
    // The list behind the editor shows the name and time too.
    getApi().bus.emitExternal(requireShowId());
}

async function addLine(): Promise<void> {
    const showData = requireShowData();
    const form = await promptLine(showData, { title: "Add Scenario Line", confirmText: "Add Line" });
    if (!form) return;
    await getApi().data.addLine({
        showId: requireShowId(),
        after: showData.lines[showData.lines.length - 1]?.id ?? null,
        lineName: form.name,
        lineNote: form.note,
        duration: form.duration,
        absoluteTime: form.absoluteTime
    });
    await refreshShowData();
}

async function editLine(lineId: string): Promise<void> {
    const showData = requireShowData();
    const line = showData.lines.find(candidate => candidate.id === lineId);
    if (!line) return;
    const form = await promptLine(showData, { title: "Edit Scenario Line", confirmText: "Save", line });
    if (!form) return;
    // Only what changed is sent, so a simultaneous edit of another field by someone else survives.
    const changes: LineEdit = {};
    if (form.name !== line.name) changes.name = form.name;
    if (form.note !== line.note.trim()) changes.note = form.note;
    if (form.duration !== line.duration) changes.duration = form.duration;
    if (form.absoluteTime !== line.absolute_time) changes.absoluteTime = form.absoluteTime;
    if (Object.keys(changes).length === 0) return;
    await getApi().data.editLine(requireShowId(), lineId, changes);
    await refreshShowData();
}

async function addComment(lineId: string, commentId: string | null = null): Promise<void> {
    const showData = requireShowData();
    const commentEdit = showData.comments.find(c => c.id === commentId);
    const commentValue = commentEdit?.comment ?? "";
    const commentRoleValue = commentEdit?.role_id ?? null;

    const roleOptions: ElementDefinition[] = [
        { type: "option", attributes: { value: "" }, text: "General (No Role)" },
        ...showData.roles.map(r => ({
            type: "option",
            attributes: { value: r.id, ...(r.id === commentRoleValue ? { selected: "selected" } : {}) },
            text: r.name
        }))
    ];

    const commentData = await dynamicPrompt({
        title: "Add Note / Comment",
        confirmText: "Save Note",
        elements: [
            { type: "select-label", id: "roleId", label: "Assign to Role", options: roleOptions },
            { type: "textarea-label", id: "commentText", label: "Comment", rows: 2, value: commentValue }
        ],
        onValidate: data => {
            if (String(data.commentText).trim().length < 1) return { commentText: "Comment can't be empty" };
            return null;
        }
    });

    if (!commentData || !commentData.commentText) return;

    const roleId = commentData.roleId ? String(commentData.roleId) : null;
    const comment = String(commentData.commentText);

    if (commentId) {
        await getApi().data.editComment(requireShowId(), commentId, roleId, comment);
    } else {
        await getApi().data.addComment(requireShowId(), lineId, roleId, comment);
    }
    await refreshShowData();
}

async function deleteLine(id: string): Promise<void> {
    await getApi().data.deleteLine(requireShowId(), id);
    await refreshShowData();
}

function toggleRoleVisibility(roleId: string, svgElement: Element): void {
    if (hiddenRoleIds.has(roleId)) {
        hiddenRoleIds.delete(roleId);
        svgElement.classList.remove("is-hidden");
    } else {
        hiddenRoleIds.add(roleId);
        svgElement.classList.add("is-hidden");
    }

    document.querySelectorAll(`[data-comment-role-id="${roleId}"]`).forEach(el => {
        el.classList.toggle("role-comment-hidden", hiddenRoleIds.has(roleId));
    });
}

async function openPrintDialog(): Promise<void> {
    const showData = requireShowData();
    const options = await dynamicPrompt({
        title: "Print & Export Run Sheet",
        confirmText: "Generate Printable View",
        elements: [
            {
                type: "select-label",
                id: "printMode",
                label: "Target Layout",
                events: {
                    change: (e: Event) => {
                        const roleSelectGroup = document.getElementById("roleSelect-group");
                        if (roleSelectGroup) {
                            roleSelectGroup.style.display = (e.target as HTMLSelectElement).value === "single_role" ? "block" : "none";
                        }
                    }
                },
                options: [
                    { type: "option", attributes: { value: "all_roles_separate" }, text: "Master Run Sheet" },
                    { type: "option", attributes: { value: "director" }, text: "Director's Overview" },
                    { type: "option", attributes: { value: "single_role" }, text: "Role Specific Sheet..." }
                ]
            },
            {
                type: "select-label",
                id: "roleSelect",
                label: "Select Role",
                options: showData.roles.map(r => ({
                    type: "option",
                    attributes: { value: r.id },
                    text: r.name
                })),
                style: { display: "none" } // Hidden on wrapper container
            }
        ]
    });

    if (!options) return;

    generateAndPrintSheet(showData, options);
}

/**
 * The printable run sheet: one A4 section per page. Cue names, notes, comments and role names are
 * written by whoever the show is shared with, so they only ever go in as `text`, never as markup.
 */
function printSheetContent(data: ShowDetails, options: PromptData): ElementDefinition[] {
    const showTitle = data.show.name || "Show Run Sheet";
    const formattedDate = new Date(data.show.time * 1000).toLocaleString();
    const generatedAt = new Date().toLocaleTimeString();

    // Helper: Identify General / Global comments
    const isGeneralComment = (comment: ScenarioLineComment): boolean => {
        if (!comment.role_id) return true;
        const role = data.roles.find(r => r.id === comment.role_id);
        return !role || role.name.toLowerCase() === "general";
    };

    const commentCells = (cell: string, comments: ScenarioLineComment[]): ElementDefinition[] => {
        if (comments.length === 0) return [{ type: "span", classes: "print-empty-text", text: "—", target: `@${cell}` }];
        return sortCommentsByRole(comments, data.roles).flatMap((comment, index): ElementDefinition[] => {
            const card = `${cell}_card${index}`;
            return [
                { type: "div", classes: "print-comment-card", varId: card, target: `@${cell}` },
                { type: "span", classes: "print-comment-role", text: data.roles.find(r => r.id === comment.role_id)?.name || "General", target: `@${card}` },
                { type: "div", classes: "print-comment-text", text: (comment.comment || "").trim(), target: `@${card}` }
            ];
        });
    };

    const columns: [string, string][] = [
        ["col-time", "Time"], ["col-duration", "Duration"], ["col-cue", "Cue"],
        ["col-note", "Note / Description"], ["col-comments", "Role Comments"]
    ];

    // One printable page sheet; `page` keeps the varIds of several sheets apart.
    const sheetSection = (page: number, titleSub: string, commentsList: ScenarioLineComment[]): ElementDefinition[] => {
        const id = (name: string) => `${name}${page}`;
        const rows = data.lines.flatMap((line, index): ElementDefinition[] => {
            const row = id(`row${index}_`);
            return [
                { type: "tr", varId: row, target: `@${id("body")}` },
                { type: "td", classes: "col-time", varId: `${row}time`, target: `@${row}` },
                { type: "strong", text: formatScenarioLineTime(resolveScenarioLineStartTimestamp(data.show.time, data.lines, index)), target: `@${row}time` },
                { type: "td", classes: "col-duration", text: formatScenarioLineDuration(line.duration), target: `@${row}` },
                { type: "td", classes: "col-cue", varId: `${row}cue`, target: `@${row}` },
                { type: "strong", text: line.name || "", target: `@${row}cue` },
                { type: "td", classes: "col-note", text: (line.note || "").trim(), target: `@${row}` },
                { type: "td", classes: "col-comments", varId: `${row}comments`, target: `@${row}` },
                ...commentCells(`${row}comments`, commentsList.filter(c => c.line_id === line.id))
            ];
        });

        const meta = (name: string, label: string, value: string): ElementDefinition[] => [
            { type: "div", varId: id(name), target: `@${id("meta")}` },
            { type: "strong", text: label, target: `@${id(name)}` },
            { type: "span", text: ` ${value}`, target: `@${id(name)}` }
        ];

        return [
            { type: "section", classes: "print-page-section", varId: id("section") },
            { type: "header", classes: "print-header", varId: id("header"), target: `@${id("section")}` },
            { type: "div", classes: "print-header-main", varId: id("headerMain"), target: `@${id("header")}` },
            { type: "h1", text: showTitle, target: `@${id("headerMain")}` },
            { type: "span", classes: "print-badge", text: titleSub, target: `@${id("headerMain")}` },
            { type: "div", classes: "print-meta-grid", varId: id("meta"), target: `@${id("header")}` },
            ...meta("date", "Date/Time:", formattedDate),
            ...meta("generated", "Generated:", generatedAt),
            { type: "table", classes: "print-table", varId: id("table"), target: `@${id("section")}` },
            { type: "thead", varId: id("head"), target: `@${id("table")}` },
            { type: "tr", varId: id("headRow"), target: `@${id("head")}` },
            ...columns.map(([classes, text]): ElementDefinition => ({ type: "th", classes, text, target: `@${id("headRow")}` })),
            { type: "tbody", varId: id("body"), target: `@${id("table")}` },
            ...rows
        ];
    };

    if (options.printMode === "all_roles_separate") {
        // Multi-page print: Includes Role Comments + General Comments for EVERY role sheet.
        // No standalone "general" page: its comments are on every role's sheet.
        return data.roles
            .filter(role => role.name.toLowerCase() !== "general")
            .flatMap((role, page) => sheetSection(page, `Role: ${role.name}`, data.comments.filter(c => c.role_id === role.id || isGeneralComment(c))));
    }
    if (options.printMode === "single_role") {
        const selectedRoleId = String(options.roleSelect);
        const roleName = data.roles.find(r => r.id === selectedRoleId)?.name ?? "Role";
        // Includes Role Comments + General Comments
        return sheetSection(0, `Role: ${roleName}`, data.comments.filter(c => c.role_id === selectedRoleId || isGeneralComment(c)));
    }
    return sheetSection(0, "Master Run Sheet", options.printMode === "director" ? data.comments : []);
}

function generateAndPrintSheet(data: ShowDetails, options: PromptData): void {
    const printContainer = document.getElementById("print-container")
        ?? dynamicGenerator({ type: "div", id: "print-container" }, document.body);
    printContainer.replaceChildren();
    generateDynamic(printSheetContent(data, options), printContainer);

    window.print();
    printContainer.remove();
}

export { openEditShow, buildEditShowLayout, printSheetContent, generateAndPrintSheet, deleteLine };
