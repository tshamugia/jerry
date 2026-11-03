import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { followSystemTheme } from "./app/theme";
import { reportLongTasks } from "./features/dev/longTasks";
import { syncSettings } from "./stores/settings";
import "./app/styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root is missing from index.html");
}

if (import.meta.env.DEV) {
  reportLongTasks();
}
followSystemTheme();
syncSettings();
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
