import { generateDynamic } from "/shared/js/dynamic_generator";

function loadHomePage() {
    generateDynamic([
        {type: "div", id: "sessionDiscovery"},
        {type: "table", target: "#sessionDiscovery", id: "discoveredSessions"},
        {type: "thead", target: "#discoveredSessions", varId: "discoveredSessionsTableBody"},
        {type: "tr", target: "@discoveredSessionsTableBody", varId: "discoveryHeaderRow"},
        {type: "th", target: "@discoveryHeaderRow", text: "Session Name"},
        {type: "th", target: "@discoveryHeaderRow", text: "Session ID"},
        {type: "th", target: "@discoveryHeaderRow", text: "Actions"},
        {type: "tbody", target: "#discoveredSessions", id: "discoveredSessionsTableBody"},
        {type: "tr", target: "#discoveredSessionsTableBody", varId: "noSessionsRow"},
        {type: "td", target: "@noSessionsRow", text: "No sessions discovered yet.", attributes: {colspan: "3"}},

        {type: "div", id: "editShows"},
        {type: "table", target: "#editShows", id: "editShowsTable"},
        {type: "thead", target: "#editShowsTable", varId: "editShowsTableHead"},
        {type: "tr", target: "@editShowsTableHead", varId: "editShowsHeaderRow"},
        {type: "th", target: "@editShowsHeaderRow", text: "Show Name"},
        {type: "th", target: "@editShowsHeaderRow", text: "Show Time"},
        {type: "th", target: "@editShowsHeaderRow", text: "Actions"},
        {type: "tbody", target: "#editShowsTable", id: "editShowsTableBody"},
        {type: "tfoot", target: "#editShowsTable", varId: "editShowsTableFoot"},
        {type: "tr", target: "@editShowsTableFoot", varId: "editShowsFooterRow"},
        {type: "td", target: "@editShowsFooterRow", classes: "create-show-cell", attributes: {colspan: "3"}, varId: "editShowsFooterCell"},
        {type: "button", target: "@editShowsFooterCell", id: "createShowBtn", children: [
            {type: "span", classes: "plus-icon", text: "+"},
            {type: "span", text: "Create Show"}
        ]}
    ], "#mainSection");
}

export { loadHomePage };