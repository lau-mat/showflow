import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event"; 
import { loadPage } from "./page_loader";

listen("session-stopped", () => {
  const button = document.querySelector("svg.btn-icon.session-active");
  button?.classList.remove("session-active");
});

loadPage("home");