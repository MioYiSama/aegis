import { createRoot } from "react-dom/client";
import { greet } from "./wasm/aegis_wasm";

function App() {
  return <p>1</p>;
}

greet("java");

const root = document.getElementById("app");
if (root) {
  createRoot(root).render(<App />);
}
