import {
  expect,
  type APIRequestContext,
} from "@playwright/test";

export type EvidenceSnapshot = {
  participant_role: "owner" | "visitor";
  canonical_playback_proven: boolean;
  av_sync_proven: boolean;
  av_sync_samples: Array<{
    request_sequence: number;
    sample_sequence: number;
    reference: string;
    absolute_offset_millis: number;
  }>;
  text_attempts: Array<{
    request_sequence: number;
    canonical_turn_sequence: number;
    canonical_output_sequence: number;
    status: string;
    failure_code: string | null;
    first_meaningful_response_millis: number | null;
    server_total_millis: number | null;
  }>;
  voice_attempts: Array<{
    request_sequence: number;
    canonical_turn_sequence: number;
    canonical_output_sequence: number;
    canonical_playback_confirmed: boolean;
    status: string;
    llm_millis: number;
    llm_first_meaningful_millis: number;
  }>;
  media_events: Array<{
    request_sequence: number | null;
    kind: string;
    elapsed_millis: number;
  }>;
};

export type BrowserJourneyState = {
  stage: "idle" | "running" | "complete" | "failed";
  error: string | null;
  ownerEvidence: EvidenceSnapshot | null;
  visitorEvidence: EvidenceSnapshot | null;
  requestedMicrophones: string[];
  interruptPayloads: string[];
};

export const waitForBrowserJourney = async (
  request: APIRequestContext,
  providerUrl: string,
): Promise<BrowserJourneyState> => {
  await expect.poll(async () => {
    const response = await request.get(`${providerUrl}/__browser-journey`);
    if (!response.ok()) return "checkpoint-unavailable";
    const state = await response.json() as BrowserJourneyState;
    return state.stage === "failed" ? `failed:${state.error ?? "unknown"}` : state.stage;
  }, { timeout: 90_000, intervals: [100, 250, 500] }).toBe("complete");

  const response = await request.get(`${providerUrl}/__browser-journey`);
  expect(response.ok()).toBeTruthy();
  return response.json() as Promise<BrowserJourneyState>;
};

const assertCommonEvidence = (snapshot: EvidenceSnapshot): void => {
  expect(snapshot.canonical_playback_proven).toBeTruthy();
  expect(snapshot.av_sync_proven).toBeTruthy();
  expect(snapshot.av_sync_samples).toHaveLength(3);
  expect(snapshot.av_sync_samples.map((sample) => sample.sample_sequence)).toEqual([1, 2, 3]);
  expect(snapshot.av_sync_samples.every((sample) =>
    sample.request_sequence === 1
      && sample.reference === "web_rtc_estimated_playout_timestamp"
      && sample.absolute_offset_millis === 60
  )).toBeTruthy();
  expect(snapshot.voice_attempts).toMatchObject([{
    request_sequence: 1,
    canonical_playback_confirmed: true,
    status: "completed",
  }]);
  const voice = snapshot.voice_attempts[0];
  expect(voice?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(voice?.canonical_output_sequence).toBeGreaterThan(0);
  expect(voice?.llm_first_meaningful_millis ?? -1).toBeLessThanOrEqual(
    voice?.llm_millis ?? -1,
  );
  expect(snapshot.media_events.some((event) =>
    event.request_sequence === 1 && event.kind === "audio_started"
  )).toBeTruthy();
};

export const assertBrowserJourneyEvidence = (journey: BrowserJourneyState): void => {
  expect(journey.stage).toBe("complete");
  expect(journey.error).toBeNull();
  expect(journey.requestedMicrophones).toContain("headset-mic");

  expect(journey.interruptPayloads).toHaveLength(1);
  const interruptPayload = JSON.parse(journey.interruptPayloads[0] ?? "{}") as {
    type?: string;
    videoId?: string;
    timestamp?: number;
  };
  expect(interruptPayload).toMatchObject({
    type: "stream/interrupt",
    videoId: "video-1",
  });
  expect(Number(interruptPayload.timestamp)).toBeGreaterThan(0);

  const owner = journey.ownerEvidence;
  expect(owner).not.toBeNull();
  if (!owner) throw new Error("OWNER_EVIDENCE_CHECKPOINT_MISSING");
  expect(owner.participant_role).toBe("owner");
  assertCommonEvidence(owner);
  expect(owner.text_attempts).toMatchObject([
    { request_sequence: 1, status: "completed", failure_code: null },
    { request_sequence: 2, status: "failed", failure_code: "PROVIDER_UNAVAILABLE" },
    { request_sequence: 3, status: "completed", failure_code: null },
  ]);
  const firstOwnerText = owner.text_attempts[0];
  expect(firstOwnerText?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(firstOwnerText?.canonical_output_sequence).toBeGreaterThan(0);
  expect(firstOwnerText?.first_meaningful_response_millis ?? -1).toBeLessThanOrEqual(
    firstOwnerText?.server_total_millis ?? -1,
  );
  expect(owner.media_events.some((event) =>
    event.request_sequence === 1 && event.kind === "interruption_stopped"
  )).toBeTruthy();
  expect(owner.media_events.some((event) =>
    event.request_sequence === null
      && event.kind === "reconnect_restored"
      && event.elapsed_millis > 0
  )).toBeTruthy();

  const visitor = journey.visitorEvidence;
  expect(visitor).not.toBeNull();
  if (!visitor) throw new Error("VISITOR_EVIDENCE_CHECKPOINT_MISSING");
  expect(visitor.participant_role).toBe("visitor");
  assertCommonEvidence(visitor);
  expect(visitor.text_attempts).toMatchObject([{
    request_sequence: 1,
    status: "completed",
  }]);
  expect(visitor.text_attempts[0]?.canonical_turn_sequence).toBeGreaterThan(0);
  expect(visitor.text_attempts[0]?.canonical_output_sequence).toBeGreaterThan(0);
};

export const assertProviderRequests = async (
  request: APIRequestContext,
  providerUrl: string,
  ownerAnswers: string[],
): Promise<void> => {
  const providerState = await request.get(`${providerUrl}/__state`);
  expect(providerState.ok()).toBeTruthy();
  const { requests } = await providerState.json() as {
    requests: Array<{
      kind: "stt" | "llm" | "avatar";
      method: string;
      path: string;
      authorization: string | null;
      contentType: string | null;
      bodyLength: number;
      bodyText: string;
    }>;
  };

  const stt = requests.filter((entry) => entry.kind === "stt");
  const llm = requests.filter((entry) => entry.kind === "llm");
  const avatar = requests.filter((entry) => entry.kind === "avatar");
  expect(stt).toHaveLength(2);
  expect(llm).toHaveLength(6);
  expect(avatar.filter((entry) => entry.path.endsWith("/streams"))).toHaveLength(2);
  expect(avatar.filter((entry) => entry.path.endsWith("/sdp"))).toHaveLength(2);
  expect(avatar.filter((entry) => entry.method === "DELETE")).toHaveLength(2);

  expect(stt.every((entry) => entry.authorization === "Bearer voice-stt-e2e-secret")).toBeTruthy();
  expect(stt.every((entry) => entry.contentType?.startsWith("multipart/form-data"))).toBeTruthy();
  expect(stt.every((entry) => entry.bodyLength > 3_000)).toBeTruthy();
  expect(llm.every((entry) => entry.authorization === "Bearer voice-llm-e2e-secret")).toBeTruthy();

  const ownerTextLlm = llm.find((entry) => entry.bodyText.includes("Текстовый вопрос владельца"));
  const ownerVoiceLlm = llm.find((entry) => entry.bodyText.includes("Привет из браузера"));
  const failedOwnerLlm = llm.find((entry) => entry.bodyText.includes("Спровоцируй отказ провайдера"));
  const recoveredOwnerLlm = llm.find((entry) => entry.bodyText.includes("Восстановление после отказа"));
  const visitorTextLlm = llm.find((entry) => entry.bodyText.includes("Текстовый вопрос visitor"));
  const visitorVoiceLlm = llm.find((entry) => entry.bodyText.includes("Что думает владелец?"));

  expect(ownerTextLlm).toBeDefined();
  expect(ownerVoiceLlm).toBeDefined();
  expect(failedOwnerLlm).toBeDefined();
  expect(recoveredOwnerLlm).toBeDefined();
  expect(visitorTextLlm).toBeDefined();
  expect(visitorVoiceLlm).toBeDefined();

  for (const ownerAnswer of ownerAnswers) {
    expect(ownerTextLlm?.bodyText).toContain(ownerAnswer);
    expect(ownerVoiceLlm?.bodyText).toContain(ownerAnswer);
    expect(failedOwnerLlm?.bodyText).toContain(ownerAnswer);
    expect(recoveredOwnerLlm?.bodyText).toContain(ownerAnswer);
    expect(visitorTextLlm?.bodyText).not.toContain(ownerAnswer);
    expect(visitorVoiceLlm?.bodyText).not.toContain(ownerAnswer);
  }
  expect(visitorTextLlm?.bodyText).toContain(
    "Visitor permissions do not expose owner-reviewed personal context",
  );
  expect(visitorVoiceLlm?.bodyText).toContain(
    "Visitor permissions do not expose owner-reviewed personal context",
  );

  expect(avatar.every((entry) => entry.authorization === "Basic voice-avatar-e2e-secret")).toBeTruthy();
  const spokenText = (streamPath: string): string =>
    avatar
      .filter((entry) => entry.method === "POST" && entry.path.endsWith(streamPath))
      .map((entry) => {
        const payload = JSON.parse(entry.bodyText) as { script?: { input?: string } };
        return payload.script?.input ?? "";
      })
      .filter(Boolean)
      .join(" ");

  expect(spokenText("/stream-1")).toContain("Голосовой ответ владельцу");
  expect(spokenText("/stream-2")).toBe("В visitor scope нет подтверждённых данных владельца");
};
