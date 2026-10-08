import { expect, test } from "@playwright/test";
import {
  authorizedSpeakText,
  replayTextFrom,
  resumeSentences,
  suggestedResumeSentence,
} from "../src/interrupted-answer.js";

test("only bounded, previously-authorized D-ID text scripts are replayable", () => {
  const command = {
    route: { kind: "live_kit_text_topic" as const, topic: "did.speak" },
    payload: JSON.stringify({ script: { type: "text", input: "Первая фраза. Вторая фраза." } }),
  };
  expect(authorizedSpeakText(command)).toBe("Первая фраза. Вторая фраза.");
  expect(authorizedSpeakText({ ...command, route: { ...command.route, topic: "did.interrupt" } })).toBeNull();
  expect(authorizedSpeakText({ ...command, payload: "{broken" })).toBeNull();
  expect(authorizedSpeakText({ ...command, payload: JSON.stringify({ script: { type: "audio", input: "http://private" } }) })).toBeNull();
  expect(authorizedSpeakText({ ...command, payload: JSON.stringify({ script: { type: "text", input: "x".repeat(16_001) } }) })).toBeNull();
});

test("resume retains exact remaining text without creating a new language model turn", () => {
  const sentences = resumeSentences([
    "Начало длинного",
    "ответа. Вторая мысль!",
    "Третья мысль без завершения",
  ]);
  expect(sentences).toEqual([
    "Начало длинного ответа.",
    "Вторая мысль!",
    "Третья мысль без завершения",
  ]);
  expect(replayTextFrom(sentences, 1)).toBe("Вторая мысль! Третья мысль без завершения");
  expect(replayTextFrom(sentences, -1)).toBeNull();
  expect(replayTextFrom(sentences, 3)).toBeNull();
  expect(resumeSentences(["x".repeat(16_001)])).toEqual([]);
});

test("automatic bookmark is explicitly conservative and user-adjustable", () => {
  const sentences = ["Первая фраза с деталями.", "Вторая фраза с деталями.", "Третья фраза."];
  expect(suggestedResumeSentence(sentences, 0)).toBe(0);
  expect(suggestedResumeSentence(sentences, Number.NaN)).toBe(0);
  expect(suggestedResumeSentence(sentences, 40_000)).toBe(2);
  expect(suggestedResumeSentence(sentences, 500)).toBe(0);
});
