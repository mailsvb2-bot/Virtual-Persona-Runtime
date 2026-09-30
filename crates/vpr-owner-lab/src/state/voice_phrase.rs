const FIRST_CLAUSE_MIN_CHARS: usize = 24;
const NEXT_CLAUSE_MIN_CHARS: usize = 48;
const FIRST_PHRASE_SOFT_LIMIT_CHARS: usize = 48;
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
        assert!(buffer.push("Да, конечно, отвечу подробно").is_empty());
        assert_eq!(
            buffer.finish().as_deref(),
            Some("Да, конечно, отвечу подробно")
        );
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
