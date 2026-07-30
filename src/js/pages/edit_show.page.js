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

    return [
        {
            type: "div",
            classes: "show-editor-container",
            children: [
                // --- SIDEBAR: Show Meta & Roles ---
                {
                    type: "aside",
                    classes: "show-editor-sidebar",
                    children: [
                        // Show Overview Card
                        {
                            type: "div",
                            classes: "editor-card",
                            children: [
                                { type: "h3", text: "Show Details" },
                                { type: "p", classes: "show-meta-time", text: `📅 ${formattedDate}` }
                            ]
                        },
                        // Roles Card
                        {
                            type: "div",
                            classes: "editor-card",
                            children: [
                                {
                                    type: "div",
                                    classes: "card-header-flex",
                                    children: [
                                        { type: "h3", text: "Roles" },
                                        { 
                                            type: "button", 
                                            classes: ["btn", "btn-action", "btn-outline-primary"], 
                                            text: "+ Add Role",
                                            events: { click: addRole }
                                        }
                                    ]
                                },
                                {
                                    type: "ul",
                                    classes: "role-list",
                                    children: data.roles.length > 0 ? data.roles.map(role => ({
                                        type: "li",
                                        classes: "role-badge",
                                        text: role.name
                                    })) : [{ type: "small", classes: "empty-msg", text: "No roles created yet." }]
                                }
                            ]
                        }
                    ]
                },

                // --- MAIN CONTENT: Scenario Lines & Comments ---
                {
                    type: "section",
                    classes: "show-editor-main",
                    children: [
                        {
                            type: "div",
                            classes: "card-header-flex",
                            children: [
                                { type: "h3", text: "Run Sheet & Scenario Lines" },
                                { 
                                    type: "button", 
                                    classes: ["btn", "btn-primary"], 
                                    text: "+ Add Line",
                                    events: { click: addLine }
                                }
                            ]
                        },
                        {
                            type: "div",
                            classes: "lines-table-wrapper",
                            children: [
                                {
                                    type: "table",
                                    classes: "lines-table",
                                    children: [
                                        {
                                            type: "thead",
                                            children: [{
                                                type: "tr",
                                                children: [
                                                    { type: "th", text: "#", style: { width: "60px" } },
                                                    { type: "th", text: "Cue / Line Content" },
                                                    { type: "th", text: "Notes & Comments" },
                                                    { type: "th", text: "Actions", style: { width: "100px", textAlign: "right" } }
                                                ]
                                            }]
                                        },
                                        {
                                            type: "tbody",
                                            children: data.lines.length > 0 ? data.lines.map(line => buildLineRow(line, data.comments, data.roles)) : [{
                                                type: "tr",
                                                children: [{
                                                    type: "td",
                                                    attributes: { colspan: "4" },
                                                    classes: "empty-table-cell",
                                                    text: "No scenario lines added yet. Click '+ Add Line' to start building your run sheet."
                                                }]
                                            }]
                                        }
                                    ]
                                }
                            ]
                        }
                    ]
                }
            ]
        }
    ];
}

/**
 * Builds a single table row for a Scenario Line and renders attached Comments inline
 */
function buildLineRow(line, allComments, allRoles) {
    const lineComments = allComments.filter(c => c.line_id === line.id);

    return {
        type: "tr",
        children: [
            { type: "td", classes: "line-number-cell", text: `#${line.line_number}` },
            { type: "td", classes: "line-content-cell", text: line.content },
            {
                type: "td",
                classes: "line-comments-cell",
                children: [
                    ...lineComments.map(comment => {
                        const assignedRole = allRoles.find(r => r.id === comment.role_id);
                        return {
                            type: "div",
                            classes: "comment-bubble",
                            children: [
                                ...(assignedRole ? [{ type: "span", classes: "comment-role-tag", text: assignedRole.name }] : []),
                                { type: "span", text: comment.comment }
                            ]
                        };
                    }),
                    {
                        type: "button",
                        classes: ["btn-text-action"],
                        text: "+ Note",
                        events: { click: () => addComment(line.id) }
                    }
                ]
            },
            {
                type: "td",
                style: { textAlign: "right" },
                children: [
                    {type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-icon-danger"], events: { click: () => deleteLine(line.id) } }
                ]
            }
        ]
    };
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
            // Side-by-side Row for Time Settings
            {
                type: "div",
                classes: "form-row-2col",
                children: [
                    {
                        type: "div",
                        classes: "form-group",
                        children: [
                            {type: "select-label", label: "Time Mode", id: "timeReference", options: [
                                { type: "option", attributes: { value: "relative" }, text: "Relative (+00:00)" },
                                { type: "option", attributes: { value: "absolute" }, text: "Absolute (Clock)" }
                            ]}
                        ]
                    },
                    {
                        type: "div",
                        classes: "form-group",
                        children: [
                            {type: "input-label", id: "timeValue", label: "Time / Offset", placeholder: "e.g., +05:00 or 20:15"}
                        ]
                    }
                ]
            },

            {type: "input-label", id: "lineName", label: "Line Name", placeholder: "e.g., Intro Cue"},
            {type: "textarea-label", id: "lineContent", label: "Cue Description / Script Line", placeholder: "Enter cue description...", rows: 3}
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