const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let currentBox = null;
let currentShowId = null;

async function openEditShow(id, name){
    currentShowId = id;
    const showContent = await invoke("get_full_show_details", {showId: id})
    console.log(showContent);

    let content = [
        {type: "button", classes: ["btn", "btn-primary"], text: "Create Role", events: {click: () => addRole()}}
    ];

    currentBox = addHoverBox(null, `Edit Show - ${name}`, content, {fullscreen: true})
}

async function addRole(){
    const newRoleData = await dynamicPrompt({
        title: "New Role",
        confirmText: "Create",
        elements: [
            {type: "label", attributes: {for: "newRoleName"}, text: "Role name"},
            {type: "input", attributes: {type: "text"}, id: "newRoleName"},
        ]
    });

    await invoke("add_role", {showId: currentShowId, roleName: newRoleData.newRoleName})
}