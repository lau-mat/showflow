const { invoke } = window.__TAURI__.core;

let currentBox = null;
let currentShowId = null;
let currentShowData = null;
const hiddenRoleIds = new Set();

async function openEditShow(id, name) {
    currentShowId = id;
    await refreshShowData();
}

async function refreshShowData() {
    currentShowData = await invoke("get_full_show_details", { showId: currentShowId });
    
    // If the box is already open, just refresh its content dynamically
    if (currentBox && currentBox.container.style.display !== "none") {
        currentBox.refreshContent(buildEditShowLayout(currentShowData));
    } else {
        currentBox = addHoverBox(
            null, 
            `Show Manager — ${currentShowData.show.name}`, 
            buildEditShowLayout(currentShowData), 
            { fullscreen: true }
        );
    }
}

/**
 * Builds the dynamic definition array for the full screen editor layout
 */
function buildEditShowLayout(data) {
    const formattedDate = new Date(data.show.time * 1000).toLocaleString();

    let content = [
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

        // 1. Dedicated flex wrapper for header buttons
        { type: "div", classes: "header-actions-group", varId: "mainHeaderActions", target: "@mainHeader" },

        // 2. Target both icons to @mainHeaderActions instead of @mainHeader
        { type: "svg", classes: ["btn-icon", "btn-outline-primary"], file: "icons/add.svg", events: { click: addLine }, target: "@mainHeaderActions" },
        { type: "svg", classes: ["btn-icon", "btn-muted"], file: "icons/print.svg", events: { click: openPrintDialog }, target: "@mainHeaderActions" },
        
        // Table Architecture
        { type: "div", classes: "lines-table-wrapper", varId: "tableWrapper", target: "@main" },
        { type: "table", classes: "lines-table", varId: "table", target: "@tableWrapper" },
        { type: "thead", varId: "tableHeader", target: "@table" },
        { type: "tr", varId: "tableHeaderRow", target: "@tableHeader" },
        { type: "th", text: "Time", style: {width: "90px"}, target: "@tableHeaderRow" },
        { type: "th", text: "Cue", style: {width: "160px"}, target: "@tableHeaderRow" },
        { type: "th", text: "Note", target: "@tableHeaderRow" },
        { type: "th", text: "Comments", style: {width: "250px"}, target: "@tableHeaderRow" },
        { type: "th", text: "Actions", style: {width: "100px", textAlign: "right"}, target: "@tableHeaderRow" },
        { type: "tbody", varId: "tableBody", target: "@table" }
    ];

    // Dynamic Roles Population (Spreading flat array)
    const roleItems = data.roles.length > 0 ?
        data.roles.flatMap(role => [
            // 1. The Badge Container
            {type: "li", classes: "role-badge", varId: `roleLi_${role.id}`,  target: "@roleCardList"},
            {type: "span", classes: "role-badge-label",  text: role.name, target: `@roleLi_${role.id}`},
            {
                type: "svg",
                file: "icons/visible.svg",
                classes: [
                    "btn-icon",
                    "btn-role-toggle",
                    hiddenRoleIds.has(role.id) ? "is-hidden" : ""
                ].filter(Boolean),
                events: {
                    click: (e) => {
                        e.stopPropagation();
                        toggleRoleVisibility(role.id, e.currentTarget);
                    }
                },
                target: `@roleLi_${role.id}`
            }
        ])
        :
        [{ type: "small", classes: "empty-msg", text: "No roles created yet.", target: "@roleCardList" }];

    // Dynamic Scenario Lines Population (Spreading flat array)
    const lineItems = data.lines.length > 0 ?
        data.lines.flatMap(line => buildLineRow(line, data.comments, data.roles)) // flatMap in case buildLineRow returns array of elements
        :
        [{
            type: "tr",
            target: "@tableBody",
            children: [{
                type: "td",
                attributes: { colspan: "5" }, // Updated colspan to 5 to match the 5 header columns
                classes: "empty-table-cell",
                text: "No scenario lines added yet. Click '+ Add Line' to start building your run sheet."
            }]
        }];

    // Flatten everything into a single level array for dynamicGenerator
    return [...content, ...roleItems, ...lineItems];
}

/**
 * Builds a single table row for a Scenario Line and renders attached Comments inline
 */
function buildLineRow(line, allComments, allRoles) {
    const lineComments = allComments.filter(c => c.line_id === line.id);

    let content = [
        {type: "tr", target: "@tableBody", data: {id: line.id}, varId: "lineRow"},
        {type: "td", classes: "line-number-cell", text: line.time_mode == 1 ? line.time : `+${line.time}`, target: "@lineRow"},
        {type: "td", classes: "line-content-cell", text: line.name, target: "@lineRow"},
        {type: "td", classes: "line-note-cell", text: line.comment, target: "@lineRow"},
        {type: "td", classes: "line-comments-cell", varId: "commentsCell", target: "@lineRow"},
        {type: "td", classes: "line-action-cell", varId: "actionCell", target: "@lineRow"},
        {type: "div", classes: "line-action-cell-wrapper", varId: "actionWrapper", target: "@actionCell"},
        {type: "svg", file: "icons/edit.svg", classes: ["btn-icon", "btn-muted"], target: "@actionWrapper"},
        {type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], events: { click: () => deleteLine(line.id) }, target: "@actionWrapper"}
    ];

    const notes = lineComments.map(comment => {
        const assignedRole = allRoles.find(r => r.id === comment.role_id);
        return {
            type: "div",
            classes: "comment-bubble",
            target: "@commentsCell",
            children: [
                ...(assignedRole ? [{ type: "span", classes: "comment-role-tag", text: assignedRole.name }] : []),
                {type: "span", text: comment.comment, classes: "comment-body-text"}
            ],
            data: {commentRoleId: comment.role_id},
            style: {cursor: "pointer"},
            events: {click: () => addComment(comment.line_id, comment.id)}
        };
    });

    const addNoteBtn = {type: "button", classes: ["btn-text-action"], text: "+ Note", events: {click: () => addComment(line.id)}, target: "@commentsCell"}

    return [...content, ...notes, addNoteBtn];
}

async function addRole() {
    const newRoleData = await dynamicPrompt({
        title: "New Role",
        confirmText: "Create",
        elements: [
            {type: "input-label", id: "newRoleName", label: "Role name"}
        ]
    });

    if (!newRoleData || !newRoleData.newRoleName) return;

    await invoke("add_role", { showId: currentShowId, roleName: newRoleData.newRoleName });
    await refreshShowData();
}

async function addLine() {
    const lineCount = currentShowData ? currentShowData.lines.length + 1 : 1;

    const lineData = await dynamicPrompt({
    title: "Add Scenario Line",
    confirmText: "Add Line",
    elements: [
        // 1. Two-Column Row for Time Controls
        {type: "div", classes: "form-row-2col", varId: "timeRow" },
        
        {type: "div", classes: "form-group", varId: "timeModeGroup", target: "@timeRow" },
        {type: "select-label", label: "Time Mode", id: "timeReference", options: [
            { type: "option", attributes: { value: "relative" }, text: "Relative (+00:00)" },
            { type: "option", attributes: { value: "absolute" }, text: "Absolute (Clock)" }
        ], target: "@timeModeGroup"},

        {type: "div", classes: "form-group", varId: "timeValueGroup", target: "@timeRow" },
        {type: "input-label", id: "timeValue", label: "Time / Offset", target: "@timeValueGroup"},

        // 2. Full-Width Main Fields (No target needed -> appends directly to modal body)
        {type: "input-label", id: "lineName", label: "Cue Name", placeholder: "e.g., Intro Cue"},
        {type: "textarea-label", id: "lineContent", label: "Cue Description", placeholder: "Enter cue description...", rows: 3}
    ]
});

    if (!lineData || !lineData.lineContent) return;

    await invoke("add_scenario_line", { 
        showId: currentShowId, 
        lineOrder: lineCount,
        lineName: lineData.lineName,
        lineComment: lineData.lineContent,
        lineTime: lineData.timeValue,
        lineTimeMode: lineData.timeReference === "relative" ? 0 : 1
    });
    await refreshShowData();
}

async function addComment(lineId, commentId = null) {
    const commentEdit = currentShowData.comments.find(c => c.id == commentId);
    const commentValue = commentEdit?.comment ?? "";
    const commentRoleValue = commentEdit?.role_id ?? null;

    const roleOptions = [
        { type: "option", attributes: { value: "" }, text: "General (No Role)" },
        ...currentShowData.roles.map(r => ({
            type: "option",
            attributes: { value: r.id, ...(r.id === commentRoleValue ? { selected: "selected" } : {}) },
            text: r.name
        }))
    ];

    const commentData = await dynamicPrompt({
        title: "Add Note / Comment",
        confirmText: "Save Note",
        elements: [
            {type: "select-label", id: "roleId", label: "Assign to Role", options: roleOptions},
            {type: "textarea-label", id: "commentText", label: "Comment", rows: "2", value: commentValue}
        ]
    });

    if (!commentData || !commentData.commentText) return;

    if(commentId){
        await invoke("edit_scenario_line_comment", {
            commentId,
            roleId: commentData.roleId ? parseInt(commentData.roleId, 10) : null,
            comment: commentData.commentText
        })
    } else {
        await invoke("add_scenario_line_comment", {
            lineId: lineId,
            roleId: commentData.roleId ? parseInt(commentData.roleId, 10) : null,
            comment: commentData.commentText
        });
    }
    await refreshShowData();
}

async function deleteLine(id){
    await invoke("delete_scenario_line", {lineId: id});

    const line = document.querySelector(`.lines-table tr[data-id='${id}']`);
    line.remove();
}

function toggleRoleVisibility(roleId, svgElement) {
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

async function openPrintDialog() {
    const options = await dynamicPrompt({
        title: "Print & Export Run Sheet",
        confirmText: "Generate Printable View",
        elements: [
            {
                type: "select-label",
                id: "printMode",
                label: "Target Layout",
                events: {
                    change: (e) => {
                        const roleSelectGroup = document.getElementById("roleSelect-group");
                        if (roleSelectGroup) {
                            roleSelectGroup.style.display = (e.target.value === "single_role") ? "block" : "none";
                        }
                    }
                },
                options: [
                    { type: "option", attributes: { value: "all_roles_separate" }, text: "Master Run Sheet (All Roles & Comments)" },
                    { type: "option", attributes: { value: "director" }, text: "Director's Cut (Cues & Notes Only)" },
                    { type: "option", attributes: { value: "single_role" }, text: "Role Specific Sheet..." }
                ]
            },
            {
                type: "select-label",
                id: "roleSelect",
                label: "Select Role",
                options: (currentShowData.roles || []).map(r => ({
                    type: "option",
                    attributes: { value: r.id },
                    text: r.name
                })),
                style: { display: "none" } // Hidden on wrapper container!
            }
        ]
    });

    if (options) {
        generateAndPrintSheet(currentShowData, options);
    }
}

function generateAndPrintSheet(data, options) {
    let printContainer = document.getElementById("print-container");
    if (!printContainer) {
        printContainer = document.createElement("div");
        printContainer.id = "print-container";
        document.body.appendChild(printContainer);
    }

    const showTitle = data.show.name || "Show Run Sheet";
    const formattedDate = new Date(data.show.time * 1000).toLocaleString();

    // Helper: Identify General / Global comments
    const isGeneralComment = (comment) => {
        if (!comment.role_id) return true;
        const role = data.roles.find(r => r.id == comment.role_id);
        return !role || role.name.toLowerCase() === "general";
    };

    // Helper to build a single printable A4 page sheet
    const buildSheetSection = (titleSub, commentsList) => {
        const tableRowsHtml = data.lines.map(line => {
            const lineComments = commentsList.filter(c => c.line_id === line.id);
            
            const commentsHtml = lineComments.length > 0 
                ? lineComments.map(c => {
                    const role = data.roles.find(r => r.id == c.role_id)?.name || "General";
                    const cleanText = (c.comment || '').trim().replaceAll('\n', '<br>');
                    return `<div class="print-comment-card"><span class="print-comment-role">${role}</span><div class="print-comment-text">${cleanText}</div></div>`;
                }).join("")
                : `<span class="print-empty-text">—</span>`;

            return `
                <tr>
                    <td class="col-time"><strong>${line.time || ''}</strong></td>
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
        fullHtml = buildSheetSection("Director Cut", []);
    } else {
        // Master Sheet (All roles + General)
        fullHtml = buildSheetSection("Master Run Sheet", data.comments);
    }

    printContainer.innerHTML = fullHtml;

    window.print();
    printContainer.remove();
}