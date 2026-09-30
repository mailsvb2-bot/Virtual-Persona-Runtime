import crypto from "node:crypto";
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

const sendDelayedEventStream = (
  response,
  firstText,
  secondText = "",
  thirdText = "",
  phraseDelayMillis = 60,
  completionDelayMillis = 40,
) => {
  response.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-store",
    "connection": "close",
  });
  response.write(
    `data: ${JSON.stringify({ choices: [{ delta: { content: firstText } }], usage: null })}\n\n`,
  );
  setTimeout(() => {
    if (secondText) {
      response.write(
        `data: ${JSON.stringify({ choices: [{ delta: { content: secondText } }], usage: null })}\n\n`,
      );
    }
    setTimeout(() => {
      if (thirdText) {
        response.write(
          `data: ${JSON.stringify({ choices: [{ delta: { content: thirdText } }], usage: null })}\n\n`,
        );
      }
      setTimeout(() => {
        response.write(
          `data: ${JSON.stringify({ choices: [], usage: { prompt_tokens: 12, completion_tokens: 12 } })}\n\n`,
        );
        response.end("data: [DONE]\n\n");
      }, completionDelayMillis);
    }, phraseDelayMillis);
  }, phraseDelayMillis);
};
const record = (kind, request, url, body) => {
  requests.push({
    kind,
    method: request.method,
    path: url.pathname,
    query: url.search,
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
    const expressiveRealtime = request.headers.authorization === "Bearer expressive-llm-e2e-secret";
    if (expressiveRealtime && prompt.includes("Привет из браузера")) {
      return sendDelayedEventStream(response, "Сначала уточню один важный момент,", " затем продолжу.", " Третья фраза.");
    }
    if (expressiveRealtime && prompt.includes("Что думает владелец?")) {
      return sendDelayedEventStream(
        response,
        "Этот ответ должен начаться,",
        " но хвост обязан отмениться.",
        " Эта фраза не должна дойти до аватара.",
        250,
        250,
      );
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

const websocketFrame = (opcode, payload = Buffer.alloc(0)) => {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload);
  if (body.length < 126) {
    return Buffer.concat([Buffer.from([0x80 | opcode, body.length]), body]);
  }
  const header = Buffer.alloc(4);
  header[0] = 0x80 | opcode;
  header[1] = 126;
  header.writeUInt16BE(body.length, 2);
  return Buffer.concat([header, body]);
};

const decodeClientFrames = (buffer) => {
  const frames = [];
  let offset = 0;
  while (buffer.length - offset >= 2) {
    const first = buffer[offset];
    const second = buffer[offset + 1];
    let length = second & 0x7f;
    let cursor = offset + 2;
    if (length === 126) {
      if (buffer.length - cursor < 2) break;
      length = buffer.readUInt16BE(cursor);
      cursor += 2;
    } else if (length === 127) {
      throw new Error("fixture does not accept oversized websocket frames");
    }
    const masked = (second & 0x80) !== 0;
    let mask = null;
    if (masked) {
      if (buffer.length - cursor < 4) break;
      mask = buffer.subarray(cursor, cursor + 4);
      cursor += 4;
    }
    if (buffer.length - cursor < length) break;
    const payload = Buffer.from(buffer.subarray(cursor, cursor + length));
    if (mask) {
      for (let index = 0; index < payload.length; index += 1) {
        payload[index] ^= mask[index % 4];
      }
    }
    frames.push({ opcode: first & 0x0f, payload });
    offset = cursor + length;
  }
  return { frames, remainder: buffer.subarray(offset) };
};

server.on("upgrade", (request, socket) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  if (url.pathname !== "/v1/listen") {
    socket.destroy();
    return;
  }
  const key = request.headers["sec-websocket-key"];
  if (typeof key !== "string") {
    socket.destroy();
    return;
  }
  const accept = crypto
    .createHash("sha1")
    .update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`)
    .digest("base64");
  socket.write([
    "HTTP/1.1 101 Switching Protocols",
    "Upgrade: websocket",
    "Connection: Upgrade",
    `Sec-WebSocket-Accept: ${accept}`,
    "",
    "",
  ].join("\r\n"));

  let pending = Buffer.alloc(0);
  let audioBytes = 0;
  socket.on("data", (chunk) => {
    pending = Buffer.concat([pending, chunk]);
    const decoded = decodeClientFrames(pending);
    pending = Buffer.from(decoded.remainder);
    for (const frame of decoded.frames) {
      if (frame.opcode === 0x2) {
        audioBytes += frame.payload.length;
        continue;
      }
      if (frame.opcode !== 0x1 || frame.payload.toString("utf8") !== '{"type":"CloseStream"}') {
        continue;
      }
      sttSequence += 1;
      requests.push({
        kind: "stt",
        method: "WEBSOCKET",
        path: url.pathname,
        query: url.search,
        authorization: request.headers.authorization ?? null,
        contentType: null,
        bodyLength: audioBytes,
        bodyText: "",
      });
      const transcript = sttSequence === 1
        ? "Привет из браузера"
        : "Что думает владелец?";
      socket.write(websocketFrame(
        0x1,
        JSON.stringify({
          type: "Results",
          is_final: false,
          channel: { alternatives: [{ transcript: "Привет" }] },
        }),
      ));
      socket.write(websocketFrame(
        0x1,
        JSON.stringify({
          type: "Results",
          is_final: true,
          channel: { alternatives: [{ transcript }] },
        }),
      ));
      socket.end(websocketFrame(0x8));
    }
  });
});

server.listen(port, "127.0.0.1", () => {
  console.log(`voice provider fixture: http://127.0.0.1:${port}`);
});

const shutdown = () => server.close(() => process.exit(0));
process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
