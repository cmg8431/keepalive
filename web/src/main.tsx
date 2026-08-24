import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./tokens.css";
import "./app.css";

// Before first paint: panel mode must not paint a canvas behind the window.
if (new URLSearchParams(location.search).has("panel")) {
  document.documentElement.classList.add("panel");
  document.body.classList.add("panel");
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
