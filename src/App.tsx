import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { lazy } from "react";
import { BrowserRouter, Route, Routes } from "react-router";

const HomePage = lazy(() => import("./pages/Home"));

const queryClient = new QueryClient();
export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <Routes>
          <Route index Component={HomePage} />
        </Routes>
      </BrowserRouter>
    </QueryClientProvider>
  );
}
