// Local design review only. Production uses the canonical cloud hostname.
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve, sep, extname } from "node:path";

const root = fileURLToPath(new URL(".", import.meta.url));
const assets = resolve(root, "site-assets") + sep;
const mime = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css",
  ".js": "application/javascript",
  ".png": "image/png",
  ".svg": "image/svg+xml",
};
createServer(async (req, res) => {
  try {
    const path = new URL(req.url, "http://localhost").pathname;
    const file =
      path === "/"
        ? resolve(root, "index.html")
        : resolve(root, "." + decodeURIComponent(path));
    if (path !== "/" && !file.startsWith(assets)) {
      res.writeHead(404);
      res.end("Not found");
      return;
    }
    let data = await readFile(file);
    if (path === "/")
      data = Buffer.from(
        data
          .toString()
          .replaceAll(
            'href="https://cloud.camofy.app/',
            'href="http://127.0.0.1:18742/',
          ),
      );
    res.writeHead(200, {
      "Content-Type": mime[extname(file)] || "application/octet-stream",
      "Cache-Control": "no-store",
    });
    res.end(data);
  } catch {
    res.writeHead(404);
    res.end("Not found");
  }
}).listen(18741, "127.0.0.1", () =>
  console.log("Camofy website preview: http://127.0.0.1:18741/"),
);
