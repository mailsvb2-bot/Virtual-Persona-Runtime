import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const files = new Map([
  ["/", "index.html"],
  ["/styles.css", "styles.css"],
  ["/app.js", "dist/app.js"],
  ["/owner-capture.js", "dist/owner-capture.js"],
  ["/evidence-export.js", "dist/evidence-export.js"],
  ["/mic-worklet.js", "mic-worklet.js"],
]);
const mime = new Map([
  [".html", "text/html; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
]);

createServer(async (request, response) => {
  const relative = files.get(request.url ?? "");
  if (!relative) {
    response.writeHead(404).end("not found");
    return;
  }
  try {
    const body = await readFile(join(root, relative));
    response.writeHead(200, { "content-type": mime.get(extname(relative)) ?? "application/octet-stream" });
    response.end(body);
  } catch {
    response.writeHead(500).end("fixture failure");
  }
}).listen(4173, "127.0.0.1");
