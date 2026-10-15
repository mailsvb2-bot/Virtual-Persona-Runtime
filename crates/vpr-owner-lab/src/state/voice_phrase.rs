const FIRST_CLAUSE_MIN_CHARS: usize = 12;
const NEXT_CLAUSE_MIN_CHARS: usize = 48;
const FIRST_PHRASE_SOFT_LIMIT_CHARS: usize = 24;
const NEXT_PHRASE_SOFT_LIMIT_CHARS: usize = 96;

#[derive(Default)]
pub(super) struct RealtimePhraseBuffer {
    pending: String,
    emitted_any: bool,
}

impl RealtimePhraseBuffer {
    pub(super) fn push(&mut self, chunk: &str) -> Vec<String> {
        self.pending.push_str(chunk);
        let mut output = Vec::new();
        while let Some(boundary) = self.next_boundary() {
            let tail = self.pending.split_off(boundary);
            let phrase = std::mem::replace(&mut self.pending, tail);
            if !phrase.trim().is_empty() {
                self.emitted_any = true;
                output.push(phrase.trim().to_owned());
            }
        }
        output
    }

    pub(super) fn finish(&mut self) -> Option<String> {
        let phrase = std::mem::take(&mut self.pending);
        let phrase = phrase.trim();
        (!phrase.is_empty()).then(|| phrase.to_owned())
    }

    fn next_boundary(&self) -> Option<usize> {
        for (index, ch) in self.pending.char_indices() {
            if matches!(ch, '.' | '!' | '?' | '…' | '\n') {
                // A period between digits is part of a date or decimal number,
                // not an audible sentence boundary. If the next LLM chunk has
                // not arrived, defer the decision instead of emitting "12.".
                if ch == '.' {
                    let previous = self.pending[..index].chars().next_back();
                    let next = self.pending[index + ch.len_utf8()..].chars().next();
                    if previous.is_some_and(|value| value.is_ascii_digit())
                        && next.is_none_or(|value| value.is_ascii_digit())
                    {
                        continue;
                    }
                }
                return Some(index + ch.len_utf8());
            }
        }

        let clause_min = if self.emitted_any {
            NEXT_CLAUSE_MIN_CHARS
        } else {
            FIRST_CLAUSE_MIN_CHARS
        };
        let mut chars_seen = 0_usize;
        for (index, ch) in self.pending.char_indices() {
            chars_seen += 1;
            if chars_seen >= clause_min && matches!(ch, ',' | ';' | ':' | '—') {
                return Some(index + ch.len_utf8());
            }
        }

        let limit = if self.emitted_any {
            NEXT_PHRASE_SOFT_LIMIT_CHARS
        } else {
            FIRST_PHRASE_SOFT_LIMIT_CHARS
        };
        if chars_seen < limit {
            return None;
        }

        chars_seen = 0;
        let mut whitespace_boundary = None;
        for (index, ch) in self.pending.char_indices() {
            chars_seen += 1;
            if ch.is_whitespace() {
                whitespace_boundary = Some(index + ch.len_utf8());
            }
            if chars_seen >= limit {
                break;
            }
        }
        whitespace_boundary
    }
}

#[cfg(test)]
mod tests {
    use super::RealtimePhraseBuffer;

    #[test]
    fn emits_sentence_before_stream_finishes() {
        let mut buffer = RealtimePhraseBuffer::default();
        assert!(buffer.push("Первая").is_empty());
        assert_eq!(buffer.push(" фраза. Вто"), ["Первая фраза."]);
        assert_eq!(buffer.finish().as_deref(), Some("Вто"));
    }

    #[test]
    fn natural_clause_boundary_releases_first_phrase_before_soft_limit() {
        let mut buffer = RealtimePhraseBuffer::default();
        assert_eq!(
            buffer.push("Сначала уточню один важный момент, затем продолжу"),
            ["Сначала уточню один важный момент,"]
        );
        assert_eq!(buffer.finish().as_deref(), Some("затем продолжу"));
    }

    #[test]
    fn tiny_intro_commas_do_not_create_choppy_phrases() {
        let mut buffer = RealtimePhraseBuffer::default();
        assert!(buffer.push("Да, ну, отвечу").is_empty());
        assert_eq!(buffer.finish().as_deref(), Some("Да, ну, отвечу"));
    }

    #[test]
    fn streams_first_audible_clause_before_llm_full_sentence() {
        let mut buffer = RealtimePhraseBuffer::default();
        let phrase = "Довольно интересный вопрос который заслуживает объяснения";
        let emitted = buffer.push(phrase);
        assert_eq!(
            emitted.len(),
            1,
            "first phrase must not wait for 48+ characters"
        );
        assert!(!emitted[0].ends_with(' '));
        assert!(emitted[0].chars().count() <= 24);
        assert!(!buffer.finish().unwrap().is_empty());
    }

    #[test]
    fn date_dots_do_not_split_russian_speech() {
        let mut buffer = RealtimePhraseBuffer::default();
        assert_eq!(
            buffer.push("Встреча назначена на 12.10.2026. Следующее событие"),
            ["Встреча назначена на 12.10.2026."]
        );
        assert_eq!(buffer.finish().as_deref(), Some("Следующее событие"));
    }

    #[test]
    fn date_split_across_llm_chunks_stays_whole() {
        let mut buffer = RealtimePhraseBuffer::default();
        assert!(buffer.push("Дата 12.").is_empty());
        assert!(buffer.push("10.").is_empty());
        assert_eq!(buffer.push("2026. Всё готово"), ["Дата 12.10.2026."]);
        assert_eq!(buffer.finish().as_deref(), Some("Всё готово"));
    }

    #[test]
    fn decimal_digits_are_not_sentence_boundaries() {
        let mut buffer = RealtimePhraseBuffer::default();
        assert_eq!(
            buffer.push("Получено 3.14 балла. Результат сохранён"),
            ["Получено 3.14 балла."]
        );
        assert_eq!(buffer.finish().as_deref(), Some("Результат сохранён"));
    }

    #[test]
    fn soft_limit_emits_at_word_boundary() {
        let mut buffer = RealtimePhraseBuffer::default();
        let output = buffer.push(
            "Это достаточно длинная первая фраза без знака завершения чтобы начать речь раньше",
        );
        assert_eq!(output.len(), 1);
        assert!(!output[0].is_empty());
        assert!(!output[0].ends_with(' '));
    }
}
