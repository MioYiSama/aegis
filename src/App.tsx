import { lazy } from "react";
import { BrowserRouter, Route, Routes } from "react-router";

const HomePage = lazy(() => import("./pages/HomePage"));

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route index element={<HomePage />} />
      </Routes>
    </BrowserRouter>
  );
}
