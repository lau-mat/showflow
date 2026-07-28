const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const currentBox = null; 

async function openEditShow(id, name){
    const showContent = await invoke("get_full_show_details", {showId: id})
    console.log(showContent);

    let content = [{type: "h1", text: "This is the edit box"}];

    addHoverBox(null, `Edit Show - ${name}`, content, {fullscreen: true})
}