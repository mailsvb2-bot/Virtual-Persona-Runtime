import http from "node:http";

const port = 18_790;
const agentId = "voice-e2e-agent";
const expressiveAgentId = "voice-e2e-expressive-agent";
const requests = [];
let streamSequence = 0;
let expressiveSessionSequence = 0;
let sttSequence = 0;
let llmSequence = 0;

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

const sendText = (response, status, contentType, body) => {
  response.writeHead(status, {
    "content-type": contentType,
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store",
  });
  response.end(body);
};
const record = (kind, request, url, body) => {
  requests.push({
    kind,
    method: request.method,
    path: url.pathname,
    authorization: request.headers.authorization ?? null,
    contentType: request.headers["content-type"] ?? null,
    bodyLength: body.length,
    bodyText: body.toString("utf8"),
  });
};

const llmEventStream = (reply) => [
  `data: ${JSON.stringify({ choices: [{ delta: { content: reply } }], usage: null })}\n\n`,
  `data: ${JSON.stringify({ choices: [], usage: { prompt_tokens: 12, completion_tokens: 4 } })}\n\n`,
  "data: [DONE]\n\n",
].join("");

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  if (request.method === "GET" && url.pathname === "/health") {
    return sendJson(response, 200, { ok: true });
  }
  if (request.method === "GET" && url.pathname === "/__state") {
    return sendJson(response, 200, { requests });
  }

  if (request.method === "GET" && url.pathname === `/agents/${agentId}`) {
    record("avatar", request, url, Buffer.alloc(0));
    return sendJson(response, 200, { presenter: { type: "clip" } });
  }
  if (request.method === "GET" && url.pathname === `/agents/${expressiveAgentId}`) {
    record("avatar", request, url, Buffer.alloc(0));
    return sendJson(response, 200, { presenter: { type: "expressive" } });
  }

  const body = await readBody(request);
  if (request.method === "POST" && url.pathname === "/v1/audio/transcriptions") {
    sttSequence += 1;
    record("stt", request, url, body);
    const text = sttSequence === 1
      ? "Привет из браузера"
      : "Что думает владелец?";
    return sendJson(response, 200, { text, language: "ru-RU" });
  }

  if (request.method === "POST" && url.pathname === "/v1/listen") {
    sttSequence += 1;
    record("stt", request, url, body);
    const transcript = sttSequence === 1
      ? "Привет из браузера"
      : "Что думает владелец?";
    return sendJson(response, 200, {
      results: {
        channels: [{
          alternatives: [{
            transcript,
            languages: ["ru"],
          }],
        }],
      },
    });
  }


  if (request.method === "POST" && url.pathname === "/v1/chat/completions") {
    llmSequence += 1;
    record("llm", request, url, body);
    const prompt = body.toString("utf8");
    if (prompt.includes("Спровоцируй отказ провайдера")) {
      return sendJson(response, 503, { error: { message: "fixture unavailable" } });
    }
    const reply = prompt.includes("Восстановление после отказа")
      ? "Ответ после восстановления"
      : prompt.includes("Текстовый вопрос владельца")
      ? "Текстовый ответ владельцу"
      : prompt.includes("Привет из браузера")
        ? "Голосовой ответ владельцу"
        : prompt.includes("Текстовый вопрос visitor")
          ? "Текстовый ответ visitor"
          : "В visitor scope нет подтверждённых данных владельца";
    return sendText(
      response,
      200,
      "text/event-stream",
      llmEventStream(reply),
    );
  }

  if (
    request.method === "POST"
    && url.pathname === `/v2/agents/${expressiveAgentId}/sessions`
  ) {
    expressiveSessionSequence += 1;
    record("avatar", request, url, body);
    return sendJson(response, 201, {
      id: `live-session-${expressiveSessionSequence}`,
      session_url: "wss://livekit.example.test",
      session_token: `fixture-livekit-token-${expressiveSessionSequence}`,
    });
  }

  if (request.method === "POST" && url.pathname === `/agents/${agentId}/streams`) {
    streamSequence += 1;
    record("avatar", request, url, body);
    return sendJson(response, 201, {
      id: `stream-${streamSequence}`,
      session_id: `session-${streamSequence}`,
      offer: { type: "offer", sdp: `v=0 voice-e2e-${streamSequence}` },
      ice_servers: [],
      fluent: true,
      interrupt_enabled: true,
    });
  }
  const streamMatch = url.pathname.match(
    new RegExp(`^/agents/${agentId}/streams/(stream-\\d+)(?:/(sdp|ice))?$`),
  );
  if (streamMatch) {
    const [, , resource] = streamMatch;
    record("avatar", request, url, body);
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
  console.log(`voice provider fixture: http://127.0.0.1:${port}`);
});

const shutdown = () => server.close(() => process.exit(0));
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
