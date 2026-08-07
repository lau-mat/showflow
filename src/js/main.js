const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let shows = null;
const btnAddShow = document.getElementById("createShowBtn");

function addShowItem(id, name, time){
  const date = new Date(time * 1000);
  const item = generateDynamic([
    {type: "tr", varId: "row"},
    {type: "td", text: name, target: "@row"},
    {type: "td", text: date.toLocaleString(), target: "@row"},
    {type: "td", varId: "action-container", target: "@row"},
    {type: "div", classes: "actions-cell", target: "@action-container", varId: "action-cell"},
    {type: "svg", file: "icons/edit.svg", classes: ["btn-icon", "btn-muted"], target: "@action-cell", varId: "editBtn"},
    {type: "svg", file: "icons/start.svg", classes: ["btn-icon", "btn-muted"], target: "@action-cell", varId: "startSession"},
    {type: "svg", file: "icons/delete.svg", classes: ["btn-icon", "btn-danger"], target: "@action-cell", varId: "delBtn" }
  ], "#editShowsTableBody");

  item.delBtn.addEventListener("click", async () => {
    await invoke("delete_show", {id});
    item.row.remove();
  });

  item.editBtn.addEventListener("click", () => openEditShow(id, name));

  item.startSession.addEventListener("click", () => startSession(id))
}

btnAddShow.addEventListener("click", async () => {
  const newShowData = await dynamicPrompt({
    title: "New Show Details",
    confirmText: "Create",
    elements: [
        {type: "input-label", id: "newShowName", label: "Show name"},
        {type: "label", attributes: {for: "newShowTime"}, text: "Show time"},
        {type: "input", attributes: {type: "datetime-local"}, id: "newShowTime"}
    ]
  });

  const unixTimestamp = Math.floor(new Date(newShowData.newShowTime).getTime() / 1000);

  const newShowId = await invoke('new_show', {name: newShowData.newShowName, time: unixTimestamp});

  shows.push({id: newShowId, name: newShowData.newShowName, time: unixTimestamp});
  addShowItem(newShowId.id, newShowData.newShowName, unixTimestamp);
});

async function startSession(id){
  const sessionResponse = await invoke("start_session", {showId: id})
  console.log(sessionResponse);
}

async function init(){
  shows = await invoke("get_shows");
  shows.forEach(show => addShowItem(show.id, show.name, show.time));
}

init();