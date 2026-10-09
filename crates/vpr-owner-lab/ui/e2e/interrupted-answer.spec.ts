import { expect, test } from "@playwright/test";
import {
  authorizedSpeakText,
  replayTextFrom,
  resumeSentences,
  resumeWordOffset,
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

test("owner selects the exact next word, never the start of the old answer", () => {
  const reply = "Первая мысль. Дальше важная мысль. Третья мысль.";
  const pos = reply.indexOf("важная");
  const word = resumeWordOffset(reply, pos);
  expect(word).toBe(Array.from(reply.slice(0, pos)).length);
  expect(Array.from(reply).slice(word!).join("")).toBe("важная мысль. Третья мысль.");
  expect(resumeWordOffset(reply, pos + 2)).toBe(word);
  expect(resumeWordOffset(reply, reply.length)).toBeNull();
  expect(resumeWordOffset(reply, -1)).toBeNull();
  expect(resumeWordOffset(reply, Number.NaN)).toBeNull();
});

test("Russian word cursor counts Unicode codepoints, not UTF-16 units", () => {
  const reply = "Привет 😊. Следующий абзац.";
  const pos = reply.indexOf("Следующий");
  expect(resumeWordOffset(reply, pos)).toBe(Array.from(reply.slice(0, pos)).length);
  expect(resumeWordOffset(reply, pos)).toBeLessThan(pos);
});
