import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./cloud.css";
import CloudApp from "./CloudApp.tsx";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <CloudApp />
    {import.meta.env.DEV && import.meta.env.MODE === "design" && (
      <div className="preview-ribbon">
        本地设计预览 · 演示数据 <a href="http://127.0.0.1:18741/">查看官网 ↗</a>
      </div>
    )}
  </StrictMode>,
);
