export const authorizedSpeakText = (command) => {
    if (command.route.topic !== "did.speak" || command.payload.length > 20_000)
        return null;
    try {
        const value = JSON.parse(command.payload);
        if (!value || typeof value !== "object" || !("script" in value))
            return null;
        const script = value.script;
        if (!script || typeof script !== "object" || !("type" in script)
            || script.type !== "text" || !("input" in script)
            || typeof script.input !== "string")
            return null;
        const input = script.input.trim();
        return input.length > 0 && input.length <= 16_000 ? input : null;
    }
    catch {
        return null;
    }
};
export const resumeSentences = (parts) => {
    if (parts.length === 0 || parts.length > 32)
        return [];
    const joined = parts.join(" ").replace(/\s+/g, " ").trim();
    if (!joined || joined.length > 16_000)
        return [];
    const sentences = [];
    let start = 0;
    const ending = /[.!?…]+(?=\s|$)/g;
    for (const match of joined.matchAll(ending)) {
        const end = (match.index ?? 0) + match[0].length;
        const phrase = joined.slice(start, end).trim();
        if (phrase)
            sentences.push(phrase);
        start = end;
    }
    const remainder = joined.slice(start).trim();
    if (remainder)
        sentences.push(remainder);
    return sentences.length <= 80 ? sentences : [];
};
export const suggestedResumeSentence = (sentences, heardMillis) => {
    if (sentences.length === 0 || !Number.isFinite(heardMillis))
        return 0;
    const conservativeCharacters = Math.max(0, heardMillis / 1000 * 11 - 16);
    let passed = 0;
    let selected = 0;
    for (let index = 0; index < sentences.length - 1; index += 1) {
        passed += (sentences[index]?.length ?? 0) + 1;
        if (passed > conservativeCharacters)
            break;
        selected = index + 1;
    }
    return selected;
};
export const replayTextFrom = (sentences, index) => {
    if (!Number.isInteger(index) || index < 0 || index >= sentences.length)
        return null;
    const text = sentences.slice(index).join(" ").trim();
    return text.length > 0 && text.length <= 16_000 ? text : null;
};
export const resumeWordOffset = (reply, utf16Cursor) => {
    if (!Number.isInteger(utf16Cursor) || utf16Cursor < 0 || utf16Cursor >= reply.length)
        return null;
    let cursor = utf16Cursor;
    while (cursor > 0 && !/[\s.,!?…:;—–()«»]/u.test(reply[cursor - 1] ?? "")) {
        cursor -= 1;
    }
    return Array.from(reply.slice(0, cursor)).length;
};
