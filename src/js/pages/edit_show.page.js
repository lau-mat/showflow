const { invoke } = window.__TAURI__.core;

let currentBox = null;
let currentShowId = null;
let currentShowData = null; // Store local state for re-rendering UI

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
        { type: "svg", classes: ["btn-icon", "btn-outline-primary"], file: "icons/add.svg", events: { click: addLine }, target: "@mainHeader" },
        
        // Table Architecture
        { type: "div", classes: "lines-table-wrapper", varId: "tableWrapper", target: "@main" },
        { type: "table", classes: "lines-table", varId: "table", target: "@tableWrapper" },
        { type: "thead", varId: "tableHeader", target: "@table" },
        { type: "tr", varId: "tableHeaderRow", target: "@tableHeader" },
        { type: "th", text: "Time", style: { width: "60px" }, target: "@tableHeaderRow" },
        { type: "th", text: "Cue", target: "@tableHeaderRow" },
        { type: "th", text: "Note", target: "@tableHeaderRow" },
        { type: "th", text: "Comments", target: "@tableHeaderRow" },
        { type: "th", text: "Actions", style: { width: "100px", textAlign: "right" }, target: "@tableHeaderRow" },
        { type: "tbody", varId: "tableBody", target: "@table" }
    ];

    // Dynamic Roles Population (Spreading flat array)
    const roleItems = data.roles.length > 0 ?
        data.roles.map(role => ({
            type: "li",
            classes: "role-badge",
            text: role.name,
            target: "@roleCardList"
        }))
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
                { type: "span", text: comment.comment }
            ]
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

async function addComment(lineId) {
    const roleOptions = [
        { type: "option", attributes: { value: "" }, text: "General (No Role)" },
        ...currentShowData.roles.map(r => ({
            type: "option",
            attributes: { value: r.id },
            text: r.name
        }))
    ];

    const commentData = await dynamicPrompt({
        title: "Add Note / Comment",
        confirmText: "Save Note",
        elements: [
            {type: "select-label", id: "roleId", label: "Assign to Role", options: roleOptions},
            {type: "textarea-label", id: "commentText", label: "Comment", rows: "2"}
        ]
    });

    if (!commentData || !commentData.commentText) return;

    await invoke("add_scenario_line_comment", {
        lineId: lineId,
        roleId: commentData.roleId ? parseInt(commentData.roleId, 10) : null,
        comment: commentData.commentText
    });
    await refreshShowData();
}

async function deleteLine(id){
    await invoke("delete_scenario_line", {lineId: id});

    const line = document.querySelector(`.lines-table tr[data-id='${id}']`);
    line.remove();
}