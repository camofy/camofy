import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./cloud.css";
import CloudApp from "./CloudApp.tsx";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <CloudApp />
  </StrictMode>,
);
