import http from "node:http";

const port = 18_792;
const channels = new Map();

const readBody = async (request) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks).toString("utf8");
};

const sendJson = (response, status, payload) => {
  const body = JSON.stringify(payload);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  response.end(body);
};

const channelFrom = (pathname) => {
  const match = pathname.match(/^\/report\/(voice|expressive)$/);
  return match?.[1] ?? null;
};

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  if (request.method === "GET" && url.pathname === "/health") {
    return sendJson(response, 200, { ok: true });
  }

  const channel = channelFrom(url.pathname);
  if (!channel) {
    return sendJson(response, 404, { ok: false, code: "NOT_FOUND" });
  }

  if (request.method === "DELETE") {
    channels.set(channel, []);
    response.writeHead(204, { "cache-control": "no-store" });
    response.end();
    return;
  }

  if (request.method === "POST") {
    try {
      const payload = JSON.parse(await readBody(request));
      const events = channels.get(channel) ?? [];
      events.push(payload);
      channels.set(channel, events);
      response.writeHead(204, { "cache-control": "no-store" });
      response.end();
    } catch {
      sendJson(response, 400, { ok: false, code: "INVALID_REPORT" });
    }
    return;
  }

  if (request.method === "GET") {
    return sendJson(response, 200, { events: channels.get(channel) ?? [] });
  }

  response.writeHead(405, { allow: "GET, POST, DELETE" });
  response.end();
});

server.listen(port, "127.0.0.1", () => {
  console.log(`journey mailbox fixture: http://127.0.0.1:${port}`);
});

const shutdown = () => server.close(() => process.exit(0));
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
