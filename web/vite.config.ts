import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { designPreview } from "./preview/plugin";

// https://vite.dev/config/
export default defineConfig(({ mode }) => ({
  plugins: [
    ...(mode === "design" ? [designPreview()] : []),
    react({
      babel: {
        plugins: [["babel-plugin-react-compiler"]],
      },
    }),
  ],
  server: {
    port: 5173,
    proxy:
      mode === "design"
        ? undefined
        : {
            "^/sub/": "http://127.0.0.1:3000",
            "/api": {
              target: "http://127.0.0.1:3000",
              changeOrigin: true,
              ws: true,
            },
          },
  },
}));
