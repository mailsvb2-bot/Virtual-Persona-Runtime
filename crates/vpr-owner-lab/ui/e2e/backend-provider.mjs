import http from "node:http";

const port = 18_788;
const agentId = "backend-e2e-agent";
const requests = [];
let streamSequence = 0;

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

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  if (request.method === "GET" && url.pathname === "/health") {
    return sendJson(response, 200, { ok: true });
  }
  if (request.method === "GET" && url.pathname === "/__state") {
    return sendJson(response, 200, { requests });
  }

  if (request.method === "GET" && url.pathname === `/agents/${agentId}`) {
    requests.push({
      method: request.method,
      path: url.pathname,
      authorization: request.headers.authorization ?? null,
      body: "",
    });
    return sendJson(response, 200, { presenter: { type: "clip" } });
  }

  const body = await readBody(request);
  requests.push({
    method: request.method,
    path: url.pathname,
    authorization: request.headers.authorization ?? null,
    body,
  });

  if (request.method === "POST" && url.pathname === `/agents/${agentId}/streams`) {
    streamSequence += 1;
    return sendJson(response, 201, {
      id: `stream-${streamSequence}`,
      session_id: `session-${streamSequence}`,
      offer: { type: "offer", sdp: `v=0 backend-e2e-${streamSequence}` },
      ice_servers: [],
    });
  }

  const streamMatch = url.pathname.match(
    new RegExp(`^/agents/${agentId}/streams/(stream-\\d+)(?:/(sdp|ice))?$`),
  );
  if (streamMatch) {
    const [, , resource] = streamMatch;
    if (request.method === "POST" && (resource === "sdp" || resource === "ice" || resource === undefined)) {
      return sendJson(response, 200, {});
    }
    if (request.method === "DELETE" && resource === undefined) {
      return sendJson(response, 200, {});
    }
  }

  return sendJson(response, 404, { ok: false, code: "NOT_FOUND" });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`backend provider fixture: http://127.0.0.1:${port}`);
});

const shutdown = () => server.close(() => process.exit(0));
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
