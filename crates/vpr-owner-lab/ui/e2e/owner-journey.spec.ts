import { expect, test, type Page, type Route } from "@playwright/test";

const csrfToken = "e2e-csrf-token";
const questions = [
  { claim_id: "opinion-working-style", prompt: "Как вы предпочитаете работать?", kind: "opinion" },
  { claim_id: "value-priority", prompt: "Что для вас важно?", kind: "value_judgment" },
  { claim_id: "preference-tone", prompt: "Какой стиль общения вы предпочитаете?", kind: "preference" },
];

type Claim = {
  claim_id: string;
  statement: string;
  kind: string;
  verification: string;
  revision: number;
  owner_reviewed: boolean;
};

type FixtureState = {
  personaId: string;
  personaVersion: number;
  captureState: null | "draft" | "captured" | "reviewed";
  answers: string[];
  claims: Claim[];
  ownerReviewed: boolean;
  sessionState: string;
  sessionAudience: null | "owner" | "visitor";
  avatarOpen: boolean;
  startAudiences: string[];
  directSpeech: string[];
  textMessages: string[];
  apiPaths: string[];
};

const installBrowserFakes = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    class FakeAudioContext {
      sampleRate = 48_000;
      audioWorklet = { addModule: async () => undefined };
      async resume(): Promise<void> {}
      async close(): Promise<void> {}
    }

    class FakePeerConnection {
      connectionState = "new";
      ontrack: ((event: unknown) => void) | null = null;
      onconnectionstatechange: (() => void) | null = null;
      onicecandidate: ((event: unknown) => void) | null = null;
      async setRemoteDescription(): Promise<void> {}
      async createAnswer(): Promise<{ type: "answer"; sdp: string }> {
        return { type: "answer", sdp: "v=0" };
      }
      async setLocalDescription(): Promise<void> {}
      close(): void { this.connectionState = "closed"; }
    }

    Object.defineProperty(window, "AudioContext", { value: FakeAudioContext });
    Object.defineProperty(window, "RTCPeerConnection", { value: FakePeerConnection });
  });
};

const json = async (route: Route, payload: unknown, status = 200): Promise<void> => {
  await route.fulfill({ status, contentType: "application/json", body: JSON.stringify(payload) });
};

const captureSnapshot = (state: FixtureState) => ({
  persona_id: state.personaId,
  persona_version: state.personaVersion,
  capture_state: state.captureState,
  answered_count: state.answers.length,
  question_count: questions.length,
  current_question: state.captureState === "draft" ? questions[state.answers.length] ?? null : null,
  claims: state.claims,
});

const statusSnapshot = (state: FixtureState) => ({
  session_state: state.sessionState,
  avatar_open: state.avatarOpen,
  egress_enabled: true,
  conversation_readiness: "text_and_voice",
  session_audience: state.sessionAudience,
  owner_context_state: state.ownerReviewed ? "reviewed" : "missing",
  persona_version: state.personaVersion,
  reviewed_owner_claims: state.sessionAudience === "visitor"
    ? 0
    : state.claims.filter((claim) => claim.owner_reviewed).length,
});

const reviewedProfile = (state: FixtureState) => ({
  persona_id: state.personaId,
  persona_version: state.personaVersion,
  claims: state.claims.map(({ claim_id, statement, kind, revision }) => ({
    claim_id, statement, kind, revision,
  })),
});

const installApiFixture = async (page: Page, state: FixtureState): Promise<void> => {
  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    state.apiPaths.push(`${request.method()} ${path}`);

    if (request.method() === "POST") {
      expect(request.headers()["x-vpr-csrf"]).toBe(csrfToken);
    }

    if (request.method() === "GET" && path === "/api/bootstrap") {
      return json(route, { csrf_token: csrfToken, egress_enabled: true });
    }
    if (request.method() === "GET" && path === "/api/status") {
      return json(route, statusSnapshot(state));
    }
    if (request.method() === "GET" && path === "/api/persona/capture") {
      if (state.captureState === null || state.ownerReviewed) {
        return json(route, {
          capture: null,
          owner_context_state: state.ownerReviewed ? "reviewed" : "missing",
          persona_version: state.personaVersion,
          reviewed_owner_claims: state.claims.filter((claim) => claim.owner_reviewed).length,
        });
      }
      return json(route, captureSnapshot(state));
    }
    if (request.method() === "GET" && path === "/api/evidence/session") {
      return json(route, { session_state: state.sessionState, fixture: true });
    }

    const body = request.postDataJSON() as Record<string, unknown>;
    if (path === "/api/persona/create") {
      state.personaId = String(body.persona_id);
      state.personaVersion = 1;
      state.captureState = "draft";
      state.answers = [];
      state.claims = [];
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/capture/answer") {
      const question = questions[state.answers.length];
      if (!question) return json(route, { ok: false, code: "INVALID_STATE_TRANSITION" }, 409);
      const statement = String(body.answer);
      state.answers.push(statement);
      state.claims.push({
        claim_id: question.claim_id,
        statement,
        kind: question.kind,
        verification: "unverified",
        revision: 1,
        owner_reviewed: false,
      });
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/capture/finish") {
      state.captureState = "captured";
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/claims/approve") {
      const claim = state.claims.find((candidate) => candidate.claim_id === String(body.claim_id));
      if (!claim) return json(route, { ok: false, code: "INVALID_INPUT" }, 400);
      claim.owner_reviewed = true;
      claim.verification = "verified";
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/claims/correct") {
      const claim = state.claims.find((candidate) => candidate.claim_id === String(body.claim_id));
      if (!claim) return json(route, { ok: false, code: "INVALID_INPUT" }, 400);
      claim.statement = String(body.statement);
      claim.kind = String(body.kind);
      claim.owner_reviewed = true;
      claim.verification = "verified";
      claim.revision += 1;
      if (state.ownerReviewed) state.personaVersion += 1;
      if (state.ownerReviewed) {
        return json(route, {
          owner_context_state: "reviewed",
          persona_version: state.personaVersion,
          reviewed_owner_claims: state.claims.length,
        });
      }
      return json(route, captureSnapshot(state));
    }
    if (path === "/api/persona/review/complete") {
      state.captureState = "reviewed";
      state.ownerReviewed = true;
      state.personaVersion = 2;
      return json(route, {
        owner_context_state: "reviewed",
        persona_version: state.personaVersion,
        reviewed_owner_claims: state.claims.length,
      });
    }
    if (path === "/api/persona/reviewed") {
      if (state.sessionAudience === "visitor") {
        return json(route, { ok: false, code: "AUTH_SCOPE_DENIED" }, 403);
      }
      return json(route, reviewedProfile(state));
    }
    if (path === "/api/avatar/start") {
      const audience = String(body.audience ?? "owner");
      state.startAudiences.push(audience);
      state.sessionAudience = audience as "owner" | "visitor";
      state.sessionState = "active";
      state.avatarOpen = true;
      return json(route, {
        evidence_session_sequence: state.startAudiences.length,
        offer: { kind: "offer", sdp: "v=0" },
        ice_servers: [],
        capabilities: ["text", "interrupt"],
      });
    }
    if (path === "/api/avatar/answer" || path === "/api/avatar/ice") {
      return json(route, { ok: true });
    }
    if (path === "/api/text/turn") {
      expect(Number(request.headers()["x-vpr-evidence-request"])).toBeGreaterThan(0);
      const text = String(body.text);
      state.textMessages.push(`${state.sessionAudience}:${text}`);
      return json(route, {
        reply: state.sessionAudience === "visitor"
          ? "Visitor scoped text reply"
          : "Owner scoped text reply",
        locale: "ru-RU",
        evidence_turn_sequence: state.textMessages.length,
        evidence_output_sequence: state.textMessages.length,
        first_meaningful_response_millis: 100,
        total_millis: 150,
      });
    }
    if (path === "/api/avatar/speak") {
      if (state.sessionAudience === "visitor") {
        return json(route, { ok: false, code: "AUTH_SCOPE_DENIED" }, 403);
      }
      state.directSpeech.push(String(body.text));
      return json(route, { ok: true });
    }
    if (path === "/api/session/revoke") {
      state.sessionState = "revoked";
      state.avatarOpen = false;
      return json(route, { ok: true });
    }
    if (path === "/api/session/close") {
      state.sessionState = "closed";
      state.sessionAudience = null;
      state.avatarOpen = false;
      return json(route, { ok: true });
    }
    return json(route, { ok: false, code: "NOT_FOUND" }, 404);
  });
};

const initialState = (): FixtureState => ({
  personaId: "",
  personaVersion: 1,
  captureState: null,
  answers: [],
  claims: [],
  ownerReviewed: false,
  sessionState: "none",
  sessionAudience: null,
  avatarOpen: false,
  startAudiences: [],
  directSpeech: [],
  textMessages: [],
  apiPaths: [],
});

test("owner review, correction, visitor scope and revoke stay connected in one browser journey", async ({ page }) => {
  const state = initialState();
  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await expect(page.getByRole("button", { name: "Подключить аватар" })).toBeDisabled();
  await page.getByLabel("Идентификатор Persona").fill("owner-e2e");
  await page.getByRole("button", { name: "Создать Persona" }).click();

  const answers = ["Люблю быстрые итерации", "Мне важна точность", "Предпочитаю спокойный тон"];
  for (const [index, answer] of answers.entries()) {
    await page.getByPlaceholder("Ответьте своими словами").fill(answer);
    await page.getByRole("button", { name: "Сохранить ответ" }).click();
    await expect(page.locator("#persona-progress")).toContainText(`${index + 1}/${questions.length}`);
  }
  const approveButtons = page.getByRole("button", { name: "Подтвердить без изменений" });
  await page.getByRole("button", { name: "Перейти к проверке" }).click();
  await expect(approveButtons).toHaveCount(questions.length);
  for (let index = 0; index < questions.length; index += 1) {
    await approveButtons.first().click();
    await expect(approveButtons).toHaveCount(questions.length - index - 1);
  }
  await page.getByRole("button", { name: "Подтвердить Persona" }).click();
  await expect(page.locator("#persona-progress")).toContainText("версия 2");
  await expect(page.getByRole("button", { name: "Подключить аватар" })).toBeEnabled();

  await page.getByRole("checkbox").check();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("WebRTC согласован");
  await page.getByLabel("Текстовый разговор").fill("Проверка owner scope");
  await page.getByRole("button", { name: "Отправить", exact: true }).click();
  await expect(page.locator("#status")).toContainText("Owner scoped text reply");
  await expect.poll(() => state.textMessages).toEqual(["owner:Проверка owner scope"]);
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");

  const ownerClaim = page.getByLabel("Текущее утверждение opinion-working-style");
  await ownerClaim.fill("Предпочитаю короткие циклы проверки");
  await page.getByRole("button", { name: "Сохранить новую редакцию" }).first().click();
  await expect(page.locator("#persona-progress")).toContainText("версия 3");
  await expect(ownerClaim).toHaveValue("Предпочитаю короткие циклы проверки");

  await page.getByLabel("Режим тестовой сессии").selectOption("visitor");
  await expect(page.locator("#persona-panel")).toBeHidden();
  await expect(page.getByLabel("Текстовый разговор")).toBeEnabled();
  await page.getByRole("button", { name: "Подключить аватар" }).click();
  await expect(page.locator("#status")).toContainText("Visitor-сессия WebRTC согласована");
  await page.getByLabel("Текстовый разговор").fill("Проверка visitor scope");
  await page.getByRole("button", { name: "Отправить", exact: true }).click();
  await expect(page.locator("#status")).toContainText("Visitor scoped text reply");
  await page.getByRole("button", { name: "Отозвать доступ" }).click();
  await expect(page.locator("#status")).toContainText("Доступ отозван");
  await page.getByRole("button", { name: "Закрыть" }).click();
  await expect(page.locator("#status")).toContainText("Сессия закрыта");

  expect(state.startAudiences).toEqual(["owner", "visitor"]);
  expect(state.textMessages).toEqual([
    "owner:Проверка owner scope",
    "visitor:Проверка visitor scope",
  ]);
  expect(state.personaVersion).toBe(3);
  expect(state.claims[0]?.revision).toBe(2);
  expect(state.apiPaths).toContain("POST /api/session/revoke");
});

test("active visitor recovery does not request owner-only persona endpoints", async ({ page }) => {
  const state = initialState();
  state.personaId = "owner-e2e";
  state.personaVersion = 3;
  state.captureState = "reviewed";
  state.ownerReviewed = true;
  state.sessionState = "active";
  state.sessionAudience = "visitor";
  state.avatarOpen = true;
  state.claims = [{
    claim_id: "opinion-working-style",
    statement: "Секретный owner-reviewed текст",
    kind: "opinion",
    verification: "verified",
    revision: 2,
    owner_reviewed: true,
  }];

  await installBrowserFakes(page);
  await installApiFixture(page, state);
  await page.goto("/");

  await expect(page.locator("#persona-panel")).toBeHidden();
  await expect(page.locator("#status")).toContainText("Найдена незакрытая сессия");
  expect(state.apiPaths).not.toContain("GET /api/persona/capture");
  expect(state.apiPaths).not.toContain("POST /api/persona/reviewed");
});
