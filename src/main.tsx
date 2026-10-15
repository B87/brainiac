import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

// The WebView's own menu (Reload, Inspect) is not part of the app. Keep it only
// where copy and paste help: text fields and the note editor. The terminal's
// hidden input is a textarea too, so it is excluded.
document.addEventListener("contextmenu", (event) => {
  const target = event.target;
  if (
    target instanceof Element &&
    !target.closest(".xterm") &&
    target.closest("input, textarea, [contenteditable], .cm-editor")
  ) {
    return;
  }
  event.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
