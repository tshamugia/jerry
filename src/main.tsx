import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { followAppearance } from "./app/appearance";
import { syncClipboard } from "./features/clipboard/clipboard";
import { reportLongTasks } from "./features/dev/longTasks";
import { syncFavorites } from "./features/sidebar/favorites";
import { syncSidebar } from "./features/sidebar/store";
import { syncAppearance } from "./stores/appearance";
import { loadKeymap } from "./stores/keymap";
import { syncSettings, useSettings } from "./stores/settings";
import { startTabs } from "./stores/tabs";
import "./app/styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root is missing from index.html");
}

if (import.meta.env.DEV) {
  reportLongTasks();
}
followAppearance();
syncSettings();
// Each pane's first tab starts opening once the settings say where, before the first render:
// subscribed before React, so this runs first.
const unsubscribe = useSettings.subscribe(({ settings }) => {
  if (settings) {
    unsubscribe();
    startTabs(settings.startup);
  }
});
syncAppearance();
syncClipboard();
syncSidebar();
syncFavorites();
void loadKeymap();
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
