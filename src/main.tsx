import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { followAppearance } from "./app/appearance";
import { syncClipboard } from "./features/clipboard/clipboard";
import { reportLongTasks } from "./features/dev/longTasks";
import { restoreSession } from "./features/session/restore";
import { saveSession } from "./features/session/save";
import { syncFavorites } from "./features/sidebar/favorites";
import { syncSidebar } from "./features/sidebar/store";
import { syncAppearance } from "./stores/appearance";
import { loadKeymap } from "./stores/keymap";
import { syncSettings } from "./stores/settings";
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
// The tabs come back before the first render (the app waits for them): the core read the
// session before the window existed, and the shown folders' listings are running already.
void restoreSession().then(() => saveSession());
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
