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
const elemCueList = document.getElementById("cueList");
const elemItemCounter = document.getElementById("itemCounter");
const btnNextCue = document.getElementById("btnNextCue");

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

btnFullscreen.addEventListener('click', async () => {
    const isFullscreen = await appWindow.isFullscreen();
    await appWindow.setFullscreen(!isFullscreen);
});

ws.on("get_show_data", data => {
    showData = data.data;

    elemShowName.innerText = showData.show.name;
    loadQRCode();
    elemItemCounter.innerText = `${showData.lines.length} Items`;

    showData.lines.forEach((line, index) => {
        generateDynamic([
            {type: "div", classes: "cue-card", varId: "wrapper"},//classes: active, next
            {type: "div", classes: "cue-info", target: "@wrapper", varId: "infoSection"},
            {type: "span", classes: "cue-title", text: `Cue ${index + 1}: ${line.name}`, target: "@infoSection"},
            {type: "span", classes: "cue-time", text: `Duration: ${line.time}`, target: "@infoSection"},
            {type: "span", text: "pending", style: {color: "var(--text-muted)", fontSize: "0.8rem"}, target: "@wrapper"} //class: live-badge
        ], elemCueList);
    });
});

ws.on("get_session_data", data => {
    sessionData = data.session;
    console.log(sessionData);
});

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