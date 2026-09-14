type ApiClient = <T>(path: string, body?: unknown) => Promise<T>;

type CaptureQuestion = {
  claim_id: string;
  prompt: string;
  kind: string;
};

type CaptureClaim = {
  claim_id: string;
  statement: string;
  kind: string;
  verification: string;
  revision: number;
  owner_reviewed: boolean;
};

type CaptureSnapshot = {
  persona_id: string;
  persona_version: number;
  capture_state: "draft" | "captured" | "reviewed";
  answered_count: number;
  question_count: number;
  current_question: CaptureQuestion | null;
  claims: CaptureClaim[];
};

type ReviewedState = {
  capture: null;
  owner_context_state: "missing" | "reviewed";
  persona_version: number;
  reviewed_owner_claims: number;
};

export type OwnerCaptureUiState = {
  reviewed: boolean;
  personaVersion: number;
  reviewedClaims: number;
};

type OwnerCaptureOptions = {
  api: ApiClient;
  onStateChange: (state: OwnerCaptureUiState) => void;
};

const byId = <T extends HTMLElement>(id: string): T => {
  const element = document.getElementById(id);
  if (!element) throw new Error(`missing element ${id}`);
  return element as T;
};

const kindLabel = (kind: string): string => {
  switch (kind) {
    case "factual": return "Факт";
    case "opinion": return "Мнение";
    case "preference": return "Предпочтение";
    case "prediction": return "Прогноз";
    case "value_judgment": return "Ценностное суждение";
    default: return kind;
  }
};

export const mountOwnerCapture = (options: OwnerCaptureOptions): { refresh: () => Promise<void> } => {
  const personaId = byId<HTMLInputElement>("persona-id");
  const createButton = byId<HTMLButtonElement>("persona-create");
  const workspace = byId<HTMLElement>("persona-workspace");
  const createRow = byId<HTMLElement>("persona-create-row");
  const progress = byId<HTMLElement>("persona-progress");
  const question = byId<HTMLElement>("persona-question");
  const answer = byId<HTMLTextAreaElement>("persona-answer");
  const answerButton = byId<HTMLButtonElement>("persona-answer-submit");
  const finishButton = byId<HTMLButtonElement>("persona-capture-finish");
  const claimsNode = byId<HTMLElement>("persona-claims");
  const completeButton = byId<HTMLButtonElement>("persona-review-complete");
  const status = byId<HTMLElement>("persona-status");

  let current: CaptureSnapshot | null = null;
  let busy = false;

  const setMessage = (text: string, state: "idle" | "ready" | "error" = "idle"): void => {
    status.textContent = text;
    status.dataset.state = state;
  };

  const emit = (reviewed: boolean, personaVersion: number, reviewedClaims: number): void => {
    options.onStateChange({ reviewed, personaVersion, reviewedClaims });
  };

  const setBusy = (value: boolean): void => {
    busy = value;
    createButton.disabled = value;
    answerButton.disabled = value || current?.current_question === null;
    finishButton.disabled = value || current === null || current.answered_count !== current.question_count;
    completeButton.disabled = value
      || current?.capture_state !== "captured"
      || !current.claims.every((claim) => claim.owner_reviewed);
    claimsNode.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
      button.disabled = value;
    });
  };

  const renderClaims = (snapshot: CaptureSnapshot): void => {
    claimsNode.replaceChildren();
    if (snapshot.capture_state === "draft") return;

    const heading = document.createElement("h3");
    heading.textContent = "Проверка утверждений";
    claimsNode.append(heading);

    for (const claim of snapshot.claims) {
      const card = document.createElement("article");
      card.className = "claim-card";

      const meta = document.createElement("div");
      meta.className = "claim-meta";
      meta.textContent = `${kindLabel(claim.kind)} · ревизия ${claim.revision} · ${claim.owner_reviewed ? "подтверждено владельцем" : "требует проверки"}`;

      const statement = document.createElement("textarea");
      statement.rows = 3;
      statement.value = claim.statement;
      statement.setAttribute("aria-label", `Утверждение ${claim.claim_id}`);

      const kind = document.createElement("select");
      kind.setAttribute("aria-label", `Тип утверждения ${claim.claim_id}`);
      for (const value of ["factual", "opinion", "preference", "prediction", "value_judgment"]) {
        const option = document.createElement("option");
        option.value = value;
        option.textContent = kindLabel(value);
        option.selected = value === claim.kind;
        kind.append(option);
      }

      const actions = document.createElement("div");
      actions.className = "row compact";

      if (!claim.owner_reviewed) {
        const approve = document.createElement("button");
        approve.className = "primary";
        approve.textContent = "Подтвердить без изменений";
        approve.addEventListener("click", () => void mutate(async () => {
          const next = await options.api<CaptureSnapshot>("/api/persona/claims/approve", {
            claim_id: claim.claim_id,
          });
          setMessage("Утверждение подтверждено", "ready");
          return next;
        }));
        actions.append(approve);
      }

      const correct = document.createElement("button");
      correct.textContent = claim.owner_reviewed ? "Сохранить новую редакцию" : "Исправить и подтвердить";
      correct.addEventListener("click", () => void mutate(async () => {
        const text = statement.value.trim();
        if (!text) throw new Error("Введите текст утверждения");
        const next = await options.api<CaptureSnapshot>("/api/persona/claims/correct", {
          claim_id: claim.claim_id,
          statement: text,
          kind: kind.value,
        });
        setMessage("Исправление сохранено как новая ревизия", "ready");
        return next;
      }));
      actions.append(correct);

      card.append(meta, statement, kind, actions);
      claimsNode.append(card);
    }
  };

  const renderCapture = (snapshot: CaptureSnapshot): void => {
    current = snapshot;
    createRow.hidden = true;
    workspace.hidden = false;
    progress.textContent = `Persona ${snapshot.persona_id} · версия ${snapshot.persona_version} · ${snapshot.answered_count}/${snapshot.question_count}`;
    const activeQuestion = snapshot.current_question;
    question.textContent = activeQuestion?.prompt ?? "Ответы собраны. Завершите сбор и проверьте утверждения.";
    answer.hidden = activeQuestion === null;
    answerButton.hidden = activeQuestion === null;
    if (activeQuestion !== null) answer.value = "";
    finishButton.hidden = snapshot.capture_state !== "draft" || snapshot.answered_count !== snapshot.question_count;
    renderClaims(snapshot);
    completeButton.hidden = snapshot.capture_state !== "captured";
    emit(snapshot.capture_state === "reviewed", snapshot.persona_version, snapshot.claims.filter((claim) => claim.owner_reviewed).length);
    setBusy(busy);
  };

  const renderReviewed = (state: ReviewedState): void => {
    current = null;
    createRow.hidden = true;
    workspace.hidden = false;
    answer.hidden = true;
    answerButton.hidden = true;
    finishButton.hidden = true;
    completeButton.hidden = true;
    claimsNode.replaceChildren();
    progress.textContent = `Persona подтверждена · версия ${state.persona_version} · утверждений ${state.reviewed_owner_claims}`;
    question.textContent = "Профиль владельца готов для канонических тестовых turn. Исправления после review создают новую PersonaVersion.";
    setMessage("Persona подтверждена владельцем", "ready");
    emit(state.owner_context_state === "reviewed", state.persona_version, state.reviewed_owner_claims);
  };

  const renderMissing = (): void => {
    current = null;
    createRow.hidden = false;
    workspace.hidden = true;
    claimsNode.replaceChildren();
    setMessage("Создайте минимальную Persona и подтвердите сведения перед разговором.");
    emit(false, 1, 0);
    setBusy(busy);
  };

  const mutate = async (operation: () => Promise<CaptureSnapshot>): Promise<void> => {
    if (busy) return;
    setBusy(true);
    try {
      renderCapture(await operation());
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Операция не выполнена", "error");
    } finally {
      setBusy(false);
    }
  };

  createButton.addEventListener("click", () => void mutate(async () => {
    const id = personaId.value.trim();
    if (!id) throw new Error("Введите идентификатор Persona");
    const snapshot = await options.api<CaptureSnapshot>("/api/persona/create", { persona_id: id });
    setMessage("Persona создана. Ответьте на вопросы владельца.", "ready");
    return snapshot;
  }));

  answerButton.addEventListener("click", () => void mutate(async () => {
    const text = answer.value.trim();
    if (!text) throw new Error("Введите ответ");
    const snapshot = await options.api<CaptureSnapshot>("/api/persona/capture/answer", { answer: text });
    setMessage("Ответ сохранён как непроверенный материал владельца", "ready");
    return snapshot;
  }));

  finishButton.addEventListener("click", () => void mutate(async () => {
    const snapshot = await options.api<CaptureSnapshot>("/api/persona/capture/finish", {});
    setMessage("Сбор завершён. Подтвердите или исправьте каждое утверждение.");
    return snapshot;
  }));

  completeButton.addEventListener("click", () => {
    if (busy) return;
    setBusy(true);
    void options.api<ReviewedState>("/api/persona/review/complete", {})
      .then((state) => renderReviewed({
        capture: null,
        owner_context_state: state.owner_context_state,
        persona_version: state.persona_version,
        reviewed_owner_claims: state.reviewed_owner_claims,
      }))
      .catch((error: unknown) => {
        setMessage(error instanceof Error ? error.message : "Review не завершён", "error");
      })
      .finally(() => setBusy(false));
  });

  const refresh = async (): Promise<void> => {
    const state = await options.api<CaptureSnapshot | ReviewedState>("/api/persona/capture");
    if ("capture" in state) {
      if (state.owner_context_state === "reviewed") renderReviewed(state);
      else renderMissing();
      return;
    }
    renderCapture(state);
  };

  renderMissing();
  return { refresh };
};
