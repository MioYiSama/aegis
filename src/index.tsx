import { createRoot } from "react-dom/client";

function App() {
  return <p>1</p>;
}

const root = document.getElementById("app");
if (root) {
  createRoot(root).render(<App />);
}
