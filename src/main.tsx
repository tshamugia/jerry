// Spike #11: first import, so its stamp runs before any other module is evaluated.
import "./features/startup/first";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { followSystemTheme } from "./app/theme";
import { useListing } from "./features/file-list/store";
import { stamp } from "./features/startup/stamps";
import "./app/styles.css";

stamp("js:main");
const root = document.getElementById("root");
if (!root) {
  throw new Error("#root is missing from index.html");
}

followSystemTheme();
// Ask for the first folder before React renders; the listing runs in Rust already.
useListing.getState().openStartup();
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
stamp("react:render_called");
