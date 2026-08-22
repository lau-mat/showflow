// Auto-detect host IP address
const host = window.location.hostname;
const ws = new WebSocket(`ws://${host}:23123`);

ws.routes = {};
let session = {
    roleId: null,
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

ws.on("get_show_data", data => {
    session.showContent = data.data

    if(session.roleId != null) return;
    if(Object.keys(session.sessionData).length !== 0) selectRole();
});

ws.on("get_session_data", data => {
    session.sessionData = data.session;

    if(session.roleId != null) return;
    if(Object.keys(session.showContent).length !== 0) selectRole();
});

ws.on("join_session", ({command, ...user}) => session.sessionData.joined_users.push(user));

async function selectRole(){
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

    ws.emit("join_session", {user_name: userName, role_id: Number(roleId)});

    session.roleId = roleId;
}