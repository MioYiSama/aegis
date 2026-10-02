import React from "react"
import { createRoot } from "react-dom/client"
import "./styles.css"
import { Providers } from "./app/providers"
import { AppRouter } from "./app/router"
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Providers>
      <AppRouter />
    </Providers>
  </React.StrictMode>,
)
