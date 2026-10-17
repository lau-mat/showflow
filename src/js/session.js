const { getCurrentWindow } = window.__TAURI__.window;
const { invoke } = window.__TAURI__.core;

// Auto-detect host IP address
const host = window.location.hostname;
const ws = new WebSocket(`ws://${host}:23123`);

let connected = false;
let showData = null;
let sessionData = null;
const appWindow = getCurrentWindow();

const elemShowName = document.getElementById("ShowName");
const btnFullscreen = document.getElementById("BtnFullscreen");
const elemItemCounter = document.getElementById("itemCounter");
const btnNextCue = document.getElementById("btnNextCue");
const btnPreviousCue = document.getElementById("previousCue");

ws.routes = {};

ws.on = (ref, func) => {
    if(typeof ref !== "string") throw new Error(`Unable to regsiter websocket event ${ref}: Ref must be a string.`)
    if(typeof func !== "function") throw new Error(`Unable to register websocket event  ${ref}: Callback must be a function`);
    ws.routes[ref] = func;
}

ws.emit = (command, data = {}) => {
    data.command = command;
    const output = JSON.stringify(data);
    ws.send(output);
}

ws.onopen = () => {
    connected = true;
    ws.emit("get_show_data");
    ws.emit("get_session_data");
};

ws.onmessage = event => {
    const data = JSON.parse(event.data);
    if(ws.routes[data.command] === undefined)
        throw new Error(`Received unknown command: ${data.command}`);
    if(typeof ws.routes[data.command] !== "function")
        throw new Error(`Received command is not a function: ${data.command}`);
    ws.routes[data.command](data);
};

ws.on("error", data => {
    throw new Error(`Backend error: ${data.message}`);
});

ws.on("cue_change", ({ current_cue, next_cue }) => {
    sessionData = sessionData || {};
    sessionData.current_cue = current_cue;
    renderCueNotes();

    // Reset all cards
    document.querySelectorAll(".cue-card").forEach(el => {
        el.classList.remove("active", "next");
        const badge = el.querySelector("span:last-child");
        if (badge) badge.textContent = "pending";
    });

    const sorted = [...showData.lines].sort((a, b) => a.order - b.order);
    const activeId = sorted[current_cue - 1]?.id;
    const nextId   = sorted[next_cue   - 1]?.id;

    const activeCard = activeId != null ? document.querySelector(`.cue-card[data-line-id="${activeId}"]`) : null;
    if (activeCard) {
        activeCard.classList.add("active");
        const badge = activeCard.querySelector("span:last-child");
        if (badge) badge.textContent = "LIVE";
        activeCard.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }

    const nextCard = nextId != null ? document.querySelector(`.cue-card[data-line-id="${nextId}"]`) : null;
    if (nextCard) nextCard.classList.add("next");
});

btnFullscreen.addEventListener('click', async () => {
    const isFullscreen = await appWindow.isFullscreen();
    await appWindow.setFullscreen(!isFullscreen);
});

ws.on("get_show_data", data => {
    showData = data.data;

    elemShowName.innerText = showData.show.name;
    renderCueNotes();
    loadQRCode();
    elemItemCounter.innerText = `${showData.lines.length} Items`;

    showData.lines.forEach(line => {
        generateDynamic([
            {type: "div", classes: "cue-card", data: {lineId: line.id}, style: {cursor: "pointer"}, events: {click: () => jumpToCue(line.id)}, varId: "wrapper"},//classes: active, next
            {type: "div", classes: "cue-info", target: "@wrapper", varId: "infoSection"},
            {type: "span", classes: "cue-title", text: `Cue ${line.id}: ${line.name}`, target: "@infoSection"},
            {type: "span", classes: "cue-time", text: `Duration: ${line.duration}s`, target: "@infoSection"},
            {type: "span", text: "pending", style: {color: "var(--text-muted)", fontSize: "0.8rem"}, target: "@wrapper"} //class: live-badge
        ], "#cueList");
    });

    showData.roles.forEach(role => {
        generateDynamic([
            {type: "div", classes: "role-item", varId: "outerWrapper"},
            {type: "div", classes: "role-details", varId: "wrapper", target: "@outerWrapper"},
            {type: "strong", text: role.name, target: "@wrapper"},
            {type: "span", classes: "role-user", text: "Users: -", data: {roleId: role.id}, target: "@wrapper"}
        ], "#role-list");
    });
});

ws.on("get_session_data", data => {
    sessionData = data.session;
    renderCueNotes();
    
    sessionData.joined_users.forEach(user => addUserToElem(user.client_role, user.client_name));
});

ws.on("join_session", data => {
    sessionData.joined_users.push(data);
    addUserToElem(data.client_role, data.client_name);
});

function addUserToElem(role_id, user_name){
    const elem = document.querySelector(`.role-user[data-role-id="${role_id}"]`);
    if(!elem) return;

    if(elem.innerText === "Users: -"){
        elem.innerText = `Users: ${user_name}`;
    } else {
        elem.innerText += `, ${user_name}`;
    }
}

function renderCueNotes() {
    if (!showData) return;

    const lines = [...showData.lines].sort((a, b) => a.order - b.order);
    const currentCue = sessionData?.current_cue;
    const currentIndex = Number.isInteger(currentCue) && currentCue > 0 ? currentCue - 1 : -1;

    renderCueNoteCard("active", lines[currentIndex] || null);
    renderCueNoteCard("next", lines[currentIndex + 1] || null);
}

function renderCueNoteCard(kind, line) {
    const number = document.getElementById(`${kind === "active" ? "current" : "next"}CueNumber`);
    const title = document.getElementById(`${kind === "active" ? "current" : "next"}CueTitle`);
    const notes = document.getElementById(`${kind === "active" ? "current" : "next"}CueNotes`);
    if (!number || !title || !notes) return;

    number.textContent = line ? `Cue ${line.id}` : "—";
    title.textContent = line ? line.name : kind === "active" ? "Waiting for cue" : "No upcoming cue";
    notes.replaceChildren();

    if (!line) {
        notes.textContent = kind === "active" ? "Waiting for the first cue." : "No upcoming cue.";
        return;
    }

    const lineComment = line.comment?.trim();
    if (lineComment) {
        const comment = document.createElement("div");
        comment.className = "note-line-comment";
        comment.textContent = lineComment;
        notes.appendChild(comment);
    }

    const comments = (showData.comments || [])
        .filter(comment => comment.line_id === line.id)
        .sort((a, b) => (a.role_id ?? -1) - (b.role_id ?? -1));

    comments.forEach(comment => {
        const roleName = comment.role_id == null
            ? "General"
            : showData.roles.find(role => role.id === comment.role_id)?.name || "Role";
        const commentElement = document.createElement("div");
        commentElement.className = "note-role-comment";

        const label = document.createElement("strong");
        label.textContent = `${roleName}: `;
        commentElement.append(label, document.createTextNode(comment.comment));
        notes.appendChild(commentElement);
    });

    if (!lineComment && comments.length === 0) {
        notes.textContent = "No notes for this cue.";
    }
}

async function loadQRCode() {
    try {
        const qrSvg = await invoke('generate_server_qr');
        document.getElementById('qr-container').innerHTML = qrSvg;
    } catch (err) {
        console.error("Failed to generate QR code:", err);
    }
}

btnNextCue.addEventListener("click", () => {
    ws.emit("next_cue")
} );

btnPreviousCue.addEventListener("click", () => {
    ws.emit("previous_cue");
});

function jumpToCue(cueId){
    ws.emit("jump_to_cue", {cue_number: cueId});
}