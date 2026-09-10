import { createRoot } from "react-dom/client";
import App from "./App";
import { StrictMode } from "react";

const root = document.getElementById("app");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
