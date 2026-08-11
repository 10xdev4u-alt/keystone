import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Router } from "./app/Router";
import { ThemeProvider } from "./theme";
import "./styles/tokens.css";
import "./styles/base.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider>
      <Router />
    </ThemeProvider>
  </StrictMode>,
);
