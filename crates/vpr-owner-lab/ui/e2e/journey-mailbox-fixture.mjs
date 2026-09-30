import http from "node:http";

const port = 18_792;
const upstreamRaw = process.env.VPR_JOURNEY_UPSTREAM;
if (!upstreamRaw) {
  throw new Error("VPR_JOURNEY_UPSTREAM is required");
}
const upstream = new URL(upstreamRaw);
const channels = new Map();

const readBody = async (request) => {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks);
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
  const match = pathname.match(/^\/__journey\/report\/(voice|expressive)$/);
  return match?.[1] ?? null;
};

const rewriteRequestHeaders = (request) => {
  const headers = { ...request.headers, host: upstream.host };
  const localOrigin = `http://127.0.0.1:${port}`;
  if (typeof headers.origin === "string" && headers.origin === localOrigin) {
    headers.origin = upstream.origin;
  }
  if (typeof headers.referer === "string" && headers.referer.startsWith(localOrigin)) {
    headers.referer = upstream.origin + headers.referer.slice(localOrigin.length);
  }
  return headers;
};

const proxy = (request, response) => {
  const target = new URL(request.url ?? "/", upstream);
  const proxied = http.request({
    protocol: upstream.protocol,
    hostname: upstream.hostname,
    port: upstream.port,
    method: request.method,
    path: `${target.pathname}${target.search}`,
    headers: rewriteRequestHeaders(request),
  }, (upstreamResponse) => {
    const headers = { ...upstreamResponse.headers };
    if (typeof headers.location === "string" && headers.location.startsWith(upstream.origin)) {
      headers.location = `http://127.0.0.1:${port}${headers.location.slice(upstream.origin.length)}`;
    }
    response.writeHead(upstreamResponse.statusCode ?? 502, headers);
    upstreamResponse.pipe(response);
  });
  proxied.on("error", (error) => {
    if (!response.headersSent) {
      sendJson(response, 502, { ok: false, code: "UPSTREAM_UNAVAILABLE", detail: error.code ?? "UNKNOWN" });
    } else {
      response.destroy(error);
    }
  });
  request.pipe(proxied);
};

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  if (request.method === "GET" && url.pathname === "/health") {
    return sendJson(response, 200, { ok: true, upstream: upstream.origin });
  }

  const channel = channelFrom(url.pathname);
  if (channel) {
    if (request.method === "DELETE") {
      channels.set(channel, []);
      response.writeHead(204, { "cache-control": "no-store" });
      response.end();
      return;
    }

    if (request.method === "POST") {
      try {
        const payload = JSON.parse((await readBody(request)).toString("utf8"));
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
    return;
  }

  proxy(request, response);
});

server.listen(port, "127.0.0.1", () => {
  console.log(`journey mailbox proxy: http://127.0.0.1:${port} -> ${upstream.origin}`);
});

const shutdown = () => server.close(() => process.exit(0));
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
