import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { followAppearance } from "./app/appearance";
import { reportLongTasks } from "./features/dev/longTasks";
import { syncAppearance } from "./stores/appearance";
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
syncAppearance();
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
