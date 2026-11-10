import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { followAppearance } from "./app/appearance";
import { syncClipboard } from "./features/clipboard/clipboard";
import { reportLongTasks } from "./features/dev/longTasks";
import { followLaunches } from "./features/session/instance";
import { restoreSession } from "./features/session/restore";
import { saveSession } from "./features/session/save";
import { syncFavorites } from "./features/sidebar/favorites";
import { syncSidebar } from "./features/sidebar/store";
import { syncAppearance } from "./stores/appearance";
import { loadKeymap } from "./stores/keymap";
import { syncSettings } from "./stores/settings";
import { syncTransfers } from "./stores/transfers";
import { syncUpdates } from "./stores/updates";
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
const restored = restoreSession();
void restored.then(() => saveSession());
// `jerry.exe <path>`: new tabs after the restored ones (#43).
followLaunches(restored);
syncAppearance();
syncClipboard();
syncSidebar();
syncFavorites();
syncUpdates();
syncTransfers();
void loadKeymap();
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
