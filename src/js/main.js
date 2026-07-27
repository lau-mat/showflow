const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const Database = window.__TAURI_PLUGIN_SQL__;

const db = await Database.load('sqlite:showflow.db');

const btnAddShow = document.getElementById("createShowBtn");
await db.execute('PRAGMA journal_mode = WAL;');
await db.execute('PRAGMA busy_timeout = 5000;');

btnAddShow.addEventListener("click", async () => {
  const newShowData = await dynamicPrompt({
    title: "New Show Details",
    confirmText: "Create",
    elements: [
        {type: "label", attributes: {for: "newShowName"}},
        {type: "input", attributes: {type: "text"}, id: "newShowName"},
        {type: "label", attributes: {for: "newShowTime"}},
        {type: "input", attributes: {type: "datetime-local"}, id: "newShowTime"}
    ]
  });

  await db.execute('INSERT INTO show (show_name, show_time) VALUES ($1, $2)', [newShowData.newShowName, newShowData.newShowTime]);

  generateDynamic([
    {type: "tr", varId: "row"},
    {type: "td", text: newShowData.newShowName, target: "@row"},
    {type: "td", text: newShowData.newShowTime, target: "@row"},
    {type: "td", children: [
        {type: "div", classes: "actions-cell", children: [
            { type: "button", text: "Edit", classes: ["btn-action", "btn-outline-muted"] },
            { type: "button", text: "Start", classes: ["btn-action", "btn-outline-primary"] },
            { type: "button", text: "Remove", classes: ["btn-action", "btn-outline-danger"] }
        ]}
    ], target: "@row"}
  ], "#editShowsTableBody");
});

const init = async () => {
    const currentContent = await db.execute("SELECT * FROM show");
    console.log(currentContent);
}

init()