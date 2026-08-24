// Auto-detect host IP address
const host = window.location.hostname;
const ws = new WebSocket(`ws://${host}:23123`);

let loginSend = false;

ws.routes = {};
let session = {
    roleId: null,
    userName: null,
    clientId: null,
    name: null,
    showContent: {},
    sessionData: {},
    personalNotes: {}
};

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

ws.onclose = () => {
    console.log("websocket closed");
}

ws.on("get_show_data", data => {
    session.showContent = data.data

    if(session.roleId != null) return;
    if(Object.keys(session.sessionData).length !== 0) selectRole();
});

ws.on("get_session_data", data => {
    session.sessionData = data.session;

    if (session.roleId != null) {
        const lines = sortedLines();
        renderCueList(lines);
        renderCurrentCue(lines);
        renderNextCue(lines);
        renderCueStrip(lines);
        return;
    }
    if(Object.keys(session.showContent).length !== 0) selectRole();
});

ws.on("join_session", ({command, ...user}) => {
    session.sessionData.joined_users.push(user);

    if(!loginSend) return;
    if(user.client_name !== session.userName || user.client_role !== session.roleId) return;
    loginSend = false;
    window.localStorage.setItem("client_id", user.client_id);
    session.clientId = user.client_id;
});

ws.on("cue_change", ({ current_cue }) => {
    session.sessionData.current_cue = current_cue;
    const lines = sortedLines();
    renderCueList(lines);
    renderCurrentCue(lines);
    renderNextCue(lines);
    renderCueStrip(lines);
});

function checkAndRestoreExistingUser(){
    const previeousClientId = window.localStorage.getItem("client_id");
    if(!previeousClientId) return null;
    return session.sessionData.joined_users.find(user => user.client_id === previeousClientId) || null;
}

async function selectRole(){
    const previousUser = checkAndRestoreExistingUser();
    if(previousUser !==  null){
        session.userName = previousUser.client_name;
        session.roleId = previousUser.client_role;
        session.clientId = previousUser.clientId;
    } else {
        const roleOptions = session.showContent.roles.map(role => ({type: "option", text: role.name, attributes: {value: role.id}}));

        const {userName, roleId} = await dynamicPrompt({ 
            title: "Select role",
            confirmText: "Join",
            required: true,
            elements: [
                {type: "input-label", label: "Name", id: "userName"},
                {type: "select-label", label: "Role", id: "roleId", options: roleOptions}
            ]
        });

        loginSend = true;
        session.roleId = Number(roleId);
        session.userName = userName;

        ws.emit("join_session", {user_name: userName, role_id: Number(roleId)});
    }

    buildStructure();
}

// ── Intervals ────────────────────────────────────────────────────────────────
let _clockInterval     = null;
let _countdownInterval = null;

// ── Helpers ───────────────────────────────────────────────────────────────────
function sortedLines() {
    return [...session.showContent.lines].sort((a, b) => a.order - b.order);
}

function currentIndex(lines) {
    const c = session.sessionData.current_cue;
    if (c == null) return -1;
    return Math.min(c - 1, lines.length - 1);
}

function getComment(lineId, roleId) {
    return (session.showContent.comments || [])
        .find(c => c.line_id === lineId && c.role_id === roleId)?.comment ?? null;
}

function parseAbsoluteSeconds(timeStr) {
    const [h, m] = timeStr.split(":").map(Number);
    const now  = new Date();
    const goal = new Date(now);
    goal.setHours(h, m, 0, 0);
    if (goal <= now) goal.setDate(goal.getDate() + 1);
    return Math.max(0, Math.floor((goal - now) / 1000));
}

function fmtCountdown(secs) {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    return h > 0
        ? `${h}:${String(m).padStart(2,"0")}:${String(s).padStart(2,"0")}`
        : `${m}:${String(s).padStart(2,"0")}`;
}

function noteNodes(lineId, small = false) {
    const general  = getComment(lineId, null);
    const personal = getComment(lineId, Number(session.roleId));
    const nodes    = [];
    const smClass  = small ? ["ch-note-sm"] : [];
    if (general) nodes.push({
        type: "div", classes: ["ch-note", "ch-note-general", ...smClass],
        children: [
            { type: "span", classes: "ch-note-label", text: "GENERAL" },
            { type: "p",    classes: "ch-note-body",  html: general.replace(/\n/g, "<br>") }
        ]
    });
    if (personal) nodes.push({
        type: "div", classes: ["ch-note", "ch-note-personal", ...smClass],
        children: [
            { type: "span", classes: "ch-note-label", text: "MY NOTE" },
            { type: "p",    classes: "ch-note-body",  html: personal.replace(/\n/g, "<br>") }
        ]
    });
    return nodes;
}

// ── Sub-renderers ─────────────────────────────────────────────────────────────
function renderCueList(lines) {
    const el = document.getElementById("chCueList");
    if (!el) return;
    cleanContainer(el);
    const cur = currentIndex(lines);
    lines.forEach((line, i) => {
        dynamicGenerator({
            type: "div",
            classes: [
                "ch-cue-item",
                ...(i < cur     ? ["ch-cue-done"]   : []),
                ...(i === cur   ? ["ch-cue-active"]  : []),
                ...(i === cur+1 ? ["ch-cue-next"]    : [])
            ],
            children: [
                { type: "span", classes: "ch-cue-num",  text: String(i + 1).padStart(2, "0") },
                { type: "span", classes: "ch-cue-name", text: line.name },
                { type: "span", classes: "ch-cue-time", text: line.time }
            ]
        }, el);
    });
}

function renderCurrentCue(lines) {
    // Update header cue counter (visible on mobile)
    const counter = document.getElementById("chCueCounter");
    if (counter) {
        const idx = currentIndex(lines);
        counter.textContent = idx >= 0 ? `${idx + 1} / ${lines.length}` : `— / ${lines.length}`;
    }

    const el = document.getElementById("chCurrentWrap");
    if (!el) return;
    cleanContainer(el);
    const idx = currentIndex(lines);
    if (idx < 0) {
        dynamicGenerator({ type: "div", classes: "ch-standby", text: "Waiting for show to start\u2026" }, el);
        return;
    }
    const line = lines[idx];
    dynamicGenerator({
        type: "div", classes: "ch-current-card",
        children: [
            { type: "div", classes: "ch-cur-header", children: [
                { type: "span", classes: "ch-cur-badge", text: `CUE ${idx + 1}` },
                { type: "h2",   classes: "ch-cur-title", text: line.name },
                { type: "span", classes: "ch-cur-time",  text: line.time }
            ]},
            ...(line.comment ? [{ type: "p", classes: "ch-cur-desc", text: line.comment }] : []),
            ...noteNodes(line.id)
        ]
    }, el);

    // Trigger brief flash to signal cue change
    const card = el.querySelector(".ch-current-card");
    if (card) {
        card.classList.remove("ch-flash");
        void card.offsetWidth; // force reflow
        card.classList.add("ch-flash");
    }
}

function renderCueStrip(lines) {
    const el = document.getElementById("chCueStrip");
    if (!el) return;
    cleanContainer(el);
    const cur = currentIndex(lines);
    lines.forEach((line, i) => {
        dynamicGenerator({
            type: "div",
            classes: [
                "ch-strip-item",
                ...(i < cur     ? ["ch-cue-done"]  : []),
                ...(i === cur   ? ["ch-cue-active"] : []),
                ...(i === cur+1 ? ["ch-cue-next"]   : [])
            ],
            children: [
                { type: "span", classes: "ch-strip-num",  text: String(i + 1).padStart(2, "0") },
                { type: "span", classes: "ch-strip-name", text: line.name }
            ]
        }, el);
    });
    // Auto-scroll active item into view
    const active = el.querySelector(".ch-cue-active");
    if (active) active.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
}

function renderNextCue(lines) {
    if (_countdownInterval) { clearInterval(_countdownInterval); _countdownInterval = null; }
    const el = document.getElementById("chNextWrap");
    if (!el) return;
    cleanContainer(el);
    const nextIdx = currentIndex(lines) + 1;
    if (nextIdx >= lines.length) return;
    const line = lines[nextIdx];
    const hasCountdown = (line.time_mode === 1);

    dynamicGenerator({
        type: "div", classes: "ch-next-card",
        children: [
            { type: "div", classes: "ch-next-header", children: [
                { type: "span", classes: "ch-next-badge", text: "NEXT" },
                { type: "span", classes: "ch-next-name",  text: line.name },
                { type: "div",  classes: "ch-next-times", children: [
                    { type: "span", classes: "ch-next-time-label", text: line.time },
                    ...(hasCountdown ? [{ type: "span", classes: "ch-countdown", id: "chCountdown", text: "" }] : [])
                ]}
            ]},
            ...noteNodes(line.id, true)
        ]
    }, el);

    if (hasCountdown) {
        const cdEl = document.getElementById("chCountdown");
        const tick = () => { if (cdEl) cdEl.textContent = fmtCountdown(parseAbsoluteSeconds(line.time)); };
        tick();
        _countdownInterval = setInterval(tick, 1000);
    }
}

// ── Layout builder ────────────────────────────────────────────────────────────
function buildStructure() {
    if (_clockInterval)     { clearInterval(_clockInterval);     _clockInterval = null; }
    if (_countdownInterval) { clearInterval(_countdownInterval); _countdownInterval = null; }

    let app = document.getElementById("chApp");
    if (!app) {
        app = document.createElement("div");
        app.id = "chApp";
        app.className = "ch-layout";
        document.body.appendChild(app);
    } else {
        cleanContainer(app);
    }

    generateDynamic([
        { type: "header", classes: "ch-header",      varId: "hdr" },
        { type: "div",    classes: "ch-show-meta",   target: "@hdr", varId: "meta" },
        { type: "span",   classes: "ch-show-name",   text: session.showContent.show.name, target: "@meta" },
        { type: "span",   classes: "ch-live-badge",  text: "LIVE", target: "@meta" },
        { type: "span",   classes: "ch-cue-counter", id: "chCueCounter", text: "— / —", target: "@meta" },
        { type: "span",   classes: "ch-clock",       id: "chClock", target: "@hdr" },

        { type: "main",   classes: "ch-body",         varId: "body" },
        { type: "aside",  classes: "ch-sidebar",      target: "@body", varId: "sidebar" },
        { type: "div",    classes: "ch-sidebar-head", text: "CUE LIST", target: "@sidebar" },
        { type: "div",    classes: "ch-cue-list",     id: "chCueList",  target: "@sidebar" },

        { type: "div",    classes: "ch-content",     target: "@body", varId: "content" },
        { type: "div",    id: "chCurrentWrap",        target: "@content" },
        { type: "div",    id: "chNextWrap",           target: "@content" },

        { type: "div",    classes: "ch-cue-strip",   id: "chCueStrip" },
    ], app);

    const lines = sortedLines();
    renderCueList(lines);
    renderCurrentCue(lines);
    renderNextCue(lines);
    renderCueStrip(lines);

    const clockEl = document.getElementById("chClock");
    const tickClock = () => { if (clockEl) clockEl.textContent = new Date().toLocaleTimeString("en-GB"); };
    tickClock();
    _clockInterval = setInterval(tickClock, 1000);
}