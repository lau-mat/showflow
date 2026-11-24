import { invoke } from "@tauri-apps/api/core";
import { addHoverBox, dynamicGenerator, dynamicPrompt, type ElementDefinition } from "/shared/js/dynamic_generator";
import { formatScenarioLineDuration, formatScenarioLineTime, resolveScenarioLineStartTimestamp } from "/shared/js/scenario_time";
import type { ScenarioLine, ScenarioLineComment, ShowDetails, ShowRole } from "/shared/js/show_types";

type PromptData = NonNullable<Awaited<ReturnType<typeof dynamicPrompt>>>;

let currentBox: ReturnType<typeof addHoverBox> | null = null;
let currentShowId: number | null = null;
let currentShowData: ShowDetails | null = null;
const hiddenRoleIds = new Set<number>();

function requireShowData(): ShowDetails {
    if (!currentShowData) throw new Error("No show loaded in the editor.");
    return currentShowData;
}

async function openEditShow(id: number): Promise<void> {
    currentShowId = id;
    await refreshShowData();
}

async function refreshShowData(): Promise<void> {
    const data = await invoke<ShowDetails>("get_full_show_details", { showId: currentShowId });
    currentShowData = data;

    // If the box is already open, just refresh its content dynamically
    if (currentBox && currentBox.container.style.display !== "none") {
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
    const formattedDate = new Date(data.show.time * 1000).toLocaleString();

    const content: ElementDefinition[] = [
        // Base Layout Containers
        { type: "div", classes: "show-editor-container", varId: "editorContainer" },
        { type: "aside", classes: "show-editor-sidebar", varId: "sidebar", target: "@editorContainer" },

        // Sidebar: Details Card
        { type: "div", classes: "editor-card", varId: "editorCard", target: "@sidebar" },
        { type: "h3", text: "Show Details", target: "@editorCard" },
        { type: "p", classes: "show-meta-time", text: formattedDate, target: "@editorCard" },

        // Sidebar: Roles Card
        { type: "div", classes: "editor-card", varId: "roleCard", target: "@sidebar" },
        { type: "div", classes: "card-header-flex", varId: "roleCardHeader", target: "@roleCard" },
        { type: "h3", text: "Roles", target: "@roleCardHeader" },
        { type: "svg", classes: ["btn-icon", "btn-outline-primary"], file: "icons/add.svg", events: { click: addRole }, target: "@roleCardHeader" },
        { type: "ul", classes: "role-list", varId: "roleCardList", target: "@roleCard" },

        // Main Area: Run Sheet
        { type: "section", classes: "show-editor-main", varId: "main", target: "@editorContainer" },
        { type: "div", classes: "card-header-flex", varId: "mainHeader", target: "@main" },
        { type: "h3", text: "Run Sheet", target: "@mainHeader" },

        // Dedicated flex wrapper for header buttons
        { type: "div", classes: "header-actions-group", varId: "mainHeaderActions", target: "@mainHeader" },
        { type: "svg", classes: ["btn-icon", "btn-outline-primary"], file: "icons/add.svg", events: { click: addLine }, target: "@mainHeaderActions" },
        { type: "svg", classes: ["btn-icon", "btn-muted"], file: "icons/print.svg", events: { click: openPrintDialog }, target: "@mainHeaderActions" },

        // Table Architecture
        { type: "div", classes: "lines-table-wrapper", varId: "tableWrapper", target: "@main" },
        { type: "table", classes: "lines-table", varId: "table", target: "@tableWrapper" },
        { type: "thead", varId: "tableHeader", target: "@table" },
        { type: "tr", varId: "tableHeaderRow", target: "@tableHeader" },
        { type: "th", text: "Time", style: { width: "110px" }, target: "@tableHeaderRow" },
        { type: "th", text: "Cue", style: { width: "160px" }, target: "@tableHeaderRow" },
        { type: "th", text: "Note", target: "@tableHeaderRow" },
        { type: "th", text: "Comments", style: { width: "250px" }, target: "@tableHeaderRow" },
        { type: "th", text: "Actions", style: { width: "100px", textAlign: "right" }, target: "@tableHeaderRow" },
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
            resolveScenarioLineStartTimestamp(data.show.time, data.lines, index)
        ))
        : [{
            type: "tr",
            target: "@tableBody",
            children: [{
                type: "td",
                attributes: { colspan: "5" },
                classes: "empty-table-cell",
                text: "No scenario lines added yet. Click '+ Add Line' to start building your run sheet."
            }]
        }];

    // Flatten everything into a single level array for dynamicGenerator
    return [...content, ...roleItems, ...lineItems];
}

/**
 * General comments (no role) first, then ascending by role id
 */
function sortCommentsByRole(comments: ScenarioLineComment[]): ScenarioLineComment[] {
    return [...comments].sort((a, b) => {
        if (a.role_id === null && b.role_id === null) return 0;
        if (a.role_id === null) return -1;
        if (b.role_id === null) return 1;
        return a.role_id - b.role_id;
    });
}

/**
 * Builds a single table row for a Scenario Line and renders attached Comments inline
 */
function buildLineRow(
    line: ScenarioLine,
    allComments: ScenarioLineComment[],
    allRoles: ShowRole[],
    startTimestamp: number | null
): ElementDefinition[] {
    const lineComments = allComments.filter(c => c.line_id === line.id);

    const content: ElementDefinition[] = [
        { type: "tr", target: "@tableBody", data: { id: line.id }, varId: "lineRow" },
        { type: "td", classes: "line-number-cell", children: [
            { type: "div", classes: "startTime", text: formatScenarioLineTime(startTimestamp) },
            { type: "div", classes: "duration", text: formatScenarioLineDuration(line.duration) }
        ], target: "@lineRow" },
        { type: "td", classes: "line-content-cell", text: line.name, target: "@lineRow" },
        { type: "td", classes: "line-note-cell", text: line.comment, target: "@lineRow" },
        { type: "td", classes: "line-comments-cell", varId: "commentsCell", target: "@lineRow" },
        { type: "td", classes: "line-action-cell", varId: "actionCell", target: "@lineRow" },
        { type: "div", classes: "line-action-cell-wrapper", varId: "actionWrapper", target: "@actionCell" },
        { type: "svg", file: "icons/edit.svg", classes: ["btn-icon", "btn-muted"], target: "@actionWrapper" },
        { type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], events: { click: () => deleteLine(line.id) }, target: "@actionWrapper" }
    ];

    const notes: ElementDefinition[] = sortCommentsByRole(lineComments).map(comment => {
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
            style: { cursor: "pointer" },
            events: { click: () => addComment(comment.line_id, comment.id) }
        };
    });

    const addNoteBtn: ElementDefinition = { type: "button", classes: ["btn-text-action"], text: "+ Note", events: { click: () => addComment(line.id) }, target: "@commentsCell" };

    return [...content, ...notes, addNoteBtn];
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

    await invoke("add_role", { showId: currentShowId, roleName: String(newRoleData.newRoleName) });
    await refreshShowData();
}

function numberOptions(count: number): ElementDefinition[] {
    return Array.from({ length: count }, (_, i) => (
        { type: "option", attributes: { value: i }, text: String(i).padStart(2, "0") }
    ));
}

function toSeconds(hours: PromptData[string], minutes: PromptData[string], seconds: PromptData[string]): number {
    return (Number(hours) * 3600) + (Number(minutes) * 60) + Number(seconds);
}

async function addLine(): Promise<void> {
    const showData = requireShowData();
    const lineCount = showData.lines.length + 1;

    const hours = numberOptions(24);
    const minutes = numberOptions(60);
    const seconds = numberOptions(60);

    function toggleAbsoluteTimeSelection(value: string): void {
        const absoluteTimeSelection = document.getElementById("absoluteTimeSelection");
        if (!absoluteTimeSelection) return;
        absoluteTimeSelection.style.display = value === "absolute" ? "flex" : "none";
    }

    const lineData = await dynamicPrompt({
        title: "Add Scenario Line",
        confirmText: "Add Line",
        elements: [
            { type: "select-label", label: "Time Mode", id: "startMode", events: { change: (e: Event) => toggleAbsoluteTimeSelection((e.target as HTMLSelectElement).value) }, options: [
                { type: "option", attributes: { value: "follow" }, text: "After scenario line" },
                { type: "option", attributes: { value: "absolute" }, text: "Fixed time" }
            ] },

            { type: "div", classes: "form-group", id: "absoluteTimeSelection", style: { display: "none" } },
            { type: "label", text: "Start Time", target: "#absoluteTimeSelection" },
            { type: "div", classes: "form-row-3col", varId: "absoluteTimeRow", target: "#absoluteTimeSelection" },
            { type: "select-label", label: "Hour", id: "timeUnit", options: hours, target: "@absoluteTimeRow" },
            { type: "select-label", label: "Minutes", id: "timeMinute", options: minutes, target: "@absoluteTimeRow" },
            { type: "select-label", label: "Seconds", id: "timeSecond", options: seconds, target: "@absoluteTimeRow" },

            { type: "div", classes: "form-group", varId: "durationGroup" },
            { type: "label", text: "Duration", target: "@durationGroup" },
            { type: "div", classes: "form-row-3col", id: "durationRow", target: "@durationGroup" },
            { type: "select-label", label: "Hour", id: "durationHour", options: hours, target: "#durationRow" },
            { type: "select-label", label: "Minutes", id: "durationMinute", options: minutes, target: "#durationRow" },
            { type: "select-label", label: "Seconds", id: "durationSecond", options: seconds, target: "#durationRow" },

            // Full-width main fields (no target -> appended directly to modal body)
            { type: "input-label", id: "lineName", label: "Cue Name", placeholder: "e.g., Intro Cue" },
            { type: "textarea-label", id: "lineContent", label: "Cue Description", placeholder: "Enter cue description...", rows: 3 }
        ],
        onValidate: data => {
            const errors: Record<string, string> = {};
            const lineName = String(data.lineName);
            if (lineName.length < 5 || lineName.length > 100)
                errors.lineName = "Cue Name must be between 5 and 100 characters.";

            if (toSeconds(data.durationHour, data.durationMinute, data.durationSecond) === 0)
                errors.durationRow = "Select the duration of this item";

            if (data.startMode === "absolute" && toSeconds(data.timeUnit, data.timeMinute, data.timeSecond) === 0)
                errors.absoluteTimeSelection = "Select the absolute start time";

            return errors;
        }
    });

    if (!lineData) return;

    const duration = toSeconds(lineData.durationHour, lineData.durationMinute, lineData.durationSecond);

    let absoluteStartTimestamp: number | null = null;
    if (lineData.startMode === "absolute") {
        const absoluteStartDate = new Date(showData.show.time * 1000);
        absoluteStartDate.setHours(
            Number(lineData.timeUnit),
            Number(lineData.timeMinute),
            Number(lineData.timeSecond),
            0
        );
        absoluteStartTimestamp = Math.floor(absoluteStartDate.getTime() / 1000);
    }

    const lineComment = String(lineData.lineContent ?? "").trim() === "" ? " " : String(lineData.lineContent);

    await invoke("add_scenario_line", {
        showId: currentShowId,
        lineOrder: lineCount,
        lineName: String(lineData.lineName),
        lineComment,
        duration,
        absoluteTime: absoluteStartTimestamp
    });
    await refreshShowData();
}

async function addComment(lineId: number, commentId: number | null = null): Promise<void> {
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

    const roleId = commentData.roleId ? parseInt(String(commentData.roleId), 10) : null;
    const comment = String(commentData.commentText);

    if (commentId) {
        await invoke("edit_scenario_line_comment", { commentId, roleId, comment });
    } else {
        await invoke("add_scenario_line_comment", { lineId, roleId, comment });
    }
    await refreshShowData();
}

async function deleteLine(id: number): Promise<void> {
    await invoke("delete_scenario_line", { lineId: id });
    await refreshShowData();
}

function toggleRoleVisibility(roleId: number, svgElement: Element): void {
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

function generateAndPrintSheet(data: ShowDetails, options: PromptData): void {
    const printContainer = document.getElementById("print-container")
        ?? dynamicGenerator({ type: "div", id: "print-container" }, document.body);

    const showTitle = data.show.name || "Show Run Sheet";
    const formattedDate = new Date(data.show.time * 1000).toLocaleString();

    // Helper: Identify General / Global comments
    const isGeneralComment = (comment: ScenarioLineComment): boolean => {
        if (!comment.role_id) return true;
        const role = data.roles.find(r => r.id === comment.role_id);
        return !role || role.name.toLowerCase() === "general";
    };

    // Helper to build a single printable A4 page sheet
    const buildSheetSection = (titleSub: string, commentsList: ScenarioLineComment[]): string => {
        const tableRowsHtml = data.lines.map((line, index) => {
            const lineComments = commentsList.filter(c => c.line_id === line.id);

            const commentsHtml = lineComments.length > 0
                ? sortCommentsByRole(lineComments).map(c => {
                    const role = data.roles.find(r => r.id === c.role_id)?.name || "General";
                    const cleanText = (c.comment || '').trim().replaceAll('\n', '<br>');
                    return `<div class="print-comment-card"><span class="print-comment-role">${role}</span><div class="print-comment-text">${cleanText}</div></div>`;
                }).join("")
                : `<span class="print-empty-text">—</span>`;

            return `
                <tr>
                    <td class="col-time"><strong>${formatScenarioLineTime(resolveScenarioLineStartTimestamp(data.show.time, data.lines, index))}</strong></td>
                    <td class="col-duration">${formatScenarioLineDuration(line.duration)}</td>
                    <td class="col-cue"><strong>${line.name || ''}</strong></td>
                    <td class="col-note">${(line.comment || '').trim().replaceAll('\n', '<br>')}</td>
                    <td class="col-comments">${commentsHtml}</td>
                </tr>
            `;
        }).join("");

        return `
            <section class="print-page-section">
                <header class="print-header">
                    <div class="print-header-main">
                        <h1>${showTitle}</h1>
                        <span class="print-badge">${titleSub}</span>
                    </div>
                    <div class="print-meta-grid">
                        <div><strong>Date/Time:</strong> ${formattedDate}</div>
                        <div><strong>Generated:</strong> ${new Date().toLocaleTimeString()}</div>
                    </div>
                </header>
                <table class="print-table">
                    <thead>
                        <tr>
                            <th class="col-time">Time</th>
                            <th class="col-duration">Duration</th>
                            <th class="col-cue">Cue</th>
                            <th class="col-note">Note / Description</th>
                            <th class="col-comments">Role Comments</th>
                        </tr>
                    </thead>
                    <tbody>
                        ${tableRowsHtml}
                    </tbody>
                </table>
            </section>
        `;
    };

    let fullHtml = "";

    if (options.printMode === "all_roles_separate") {
        // Multi-page print: Includes Role Comments + General Comments for EVERY role sheet
        data.roles.forEach(role => {
            if (role.name.toLowerCase() === "general") return; // Skip standalone general page if iterating roles

            const roleComments = data.comments.filter(c => c.role_id === role.id || isGeneralComment(c));
            fullHtml += buildSheetSection(`Role: ${role.name}`, roleComments);
        });
    } else if (options.printMode === "single_role") {
        const selectedRoleId = Number(options.roleSelect);
        const role = data.roles.find(r => r.id === selectedRoleId);
        const roleName = role ? role.name : "Role";

        // Includes Role Comments + General Comments
        const roleComments = data.comments.filter(c => c.role_id === selectedRoleId || isGeneralComment(c));
        fullHtml = buildSheetSection(`Role: ${roleName}`, roleComments);
    } else if (options.printMode === "director") {
        fullHtml = buildSheetSection("Master Run Sheet", data.comments);
    } else {
        fullHtml = buildSheetSection("Master Run Sheet", []);
    }

    printContainer.innerHTML = fullHtml;

    window.print();
    printContainer.remove();
}

export { openEditShow, buildEditShowLayout, generateAndPrintSheet, deleteLine };
