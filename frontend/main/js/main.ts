const isApp = isTauriProtocol();
window.localStorage.setItem("isApp", JSON.stringify(isApp));

async function bootstrap(): Promise<void> {
  if (isApp) {
    await import("./app.ts");
  } else {
    await import("./browser.ts");
  }
}

bootstrap().catch(console.error);

export function isTauriProtocol(): boolean {
  if (typeof window === 'undefined') {
    return false; // Running server-side or in SSR
  }

  // Tauri v1 and v2 injects these global objects into the webview
  return Boolean(
    (window as any).__TAURI__ || 
    (window as any).__TAURI_INTERNALS__
  );
}

let shows: {id: number, name: string, time: number}[] = [];
const btnAddShow = document.getElementById("createShowBtn");

function addShowItem(id: number, name: string, time: number){
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

  item.editBtn.addEventListener("click", () => openEditShow(id));

  item.startSession.addEventListener("click", e => startSession(id, item.startSession))
}

btnAddShow?.addEventListener("click", async () => {
  const newShowData = await dynamicPrompt({
    title: "New Show Details",
    confirmText: "Create",
    elements: [
        {type: "input-label", id: "newShowName", label: "Show name"},
        {type: "label", attributes: {for: "newShowTime"}, text: "Show time"},
        {type: "input", attributes: {type: "datetime-local"}, id: "newShowTime"}
    ],
    onValidate: data => {
      let errors = {};
      if(!data.newShowName) errors.newShowName = "Please fill in a name";
      if(!data.newShowTime) errors.newShowTime = "A start time must be selected";
      return errors;
    }
  });

  if(!newShowData) return;

  const unixTimestamp = Math.floor(new Date(newShowData.newShowTime).getTime() / 1000);

  const newShowId = await invoke('new_show', {name: newShowData.newShowName, time: unixTimestamp}) as {id: number};

  shows.push({id: newShowId, name: newShowData.newShowName, time: unixTimestamp});
  addShowItem(newShowId.id, newShowData.newShowName, unixTimestamp);
});

async function startSession(id: number, button: HTMLElement){
  const sessionResponse = await invoke("start_session", {showId: id})
  button.classList.add("session-active");
}

async function init(){
  shows = await invoke("get_shows") as {id: number, name: string, time: number}[];
  shows.forEach(show => addShowItem(show.id, show.name, show.time));
}

//init();