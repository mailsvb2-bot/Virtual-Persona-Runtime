use std::collections::VecDeque;

const DEFAULT_MAX_PHRASE_CHARS: usize = 120;

#[derive(Debug)]
pub struct SpokenPhraseBuffer {
    pending: String,
    ready: VecDeque<String>,
    max_phrase_chars: usize,
}

impl Default for SpokenPhraseBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PHRASE_CHARS)
    }
}

impl SpokenPhraseBuffer {
    #[must_use]
    pub fn new(max_phrase_chars: usize) -> Self {
        Self {
            pending: String::new(),
            ready: VecDeque::new(),
            max_phrase_chars: max_phrase_chars.max(24),
        }
    }

    pub fn push_chunk(&mut self, chunk: &str) {
        self.pending.push_str(chunk);
        self.extract_ready(false);
    }

    pub fn pop_ready(&mut self) -> Option<String> {
        self.ready.pop_front()
    }

    pub fn finish(&mut self) -> Option<String> {
        self.extract_ready(true);
        self.pop_ready()
    }

    fn extract_ready(&mut self, final_flush: bool) {
        loop {
            let boundary = sentence_boundary(&self.pending)
                .or_else(|| length_boundary(&self.pending, self.max_phrase_chars));
            let Some(end) = boundary else {
                break;
            };
            self.queue_prefix(end);
        }
        if final_flush && !self.pending.trim().is_empty() {
            let end = self.pending.len();
            self.queue_prefix(end);
        }
    }

    fn queue_prefix(&mut self, end: usize) {
        let remainder = self.pending.split_off(end);
        let phrase = self.pending.trim().to_owned();
        self.pending = remainder;
        if !phrase.is_empty() {
            self.ready.push_back(phrase);
        }
    }
}

fn sentence_boundary(text: &str) -> Option<usize> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (index, (byte_index, ch)) in chars.iter().copied().enumerate() {
        if !matches!(ch, '.' | '!' | '?' | '…' | '\n') {
            continue;
        }
        if ch == '\n' {
            return Some(byte_index + ch.len_utf8());
        }
        let next = chars.get(index + 1).map(|(_, ch)| *ch);
        if next.is_none_or(char::is_whitespace)
            || next.is_some_and(|ch| matches!(ch, '»' | '”' | '"' | ')' | ']' | '}'))
        {
            let mut end = byte_index + ch.len_utf8();
            for (_, trailing) in chars.iter().skip(index + 1) {
                if matches!(trailing, '»' | '”' | '"' | ')' | ']' | '}') {
                    end += trailing.len_utf8();
                } else {
                    break;
                }
            }
            return Some(end);
        }
    }
    None
}

fn length_boundary(text: &str, max_chars: usize) -> Option<usize> {
    let mut count = 0;
    let mut last_whitespace_end = None;
    for (byte_index, ch) in text.char_indices() {
        count += 1;
        if ch.is_whitespace() {
            last_whitespace_end = Some(byte_index + ch.len_utf8());
        }
        if count >= max_chars {
            return last_whitespace_end.or(Some(byte_index + ch.len_utf8()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_completed_sentence_without_waiting_for_stream_end() {
        let mut buffer = SpokenPhraseBuffer::default();
        buffer.push_chunk("Первая фра");
        assert_eq!(buffer.pop_ready(), None);
        buffer.push_chunk("за. Вторая ещё идёт");
        assert_eq!(buffer.pop_ready().as_deref(), Some("Первая фраза."));
        assert_eq!(buffer.pop_ready(), None);
        assert_eq!(buffer.finish().as_deref(), Some("Вторая ещё идёт"));
    }

    #[test]
    fn preserves_multiple_ready_sentences_from_one_provider_chunk() {
        let mut buffer = SpokenPhraseBuffer::default();
        buffer.push_chunk("Да. Конечно! Продолжаю");
        assert_eq!(buffer.pop_ready().as_deref(), Some("Да."));
        assert_eq!(buffer.pop_ready().as_deref(), Some("Конечно!"));
        assert_eq!(buffer.finish().as_deref(), Some("Продолжаю"));
    }

    #[test]
    fn bounds_unpunctuated_generation_on_word_boundary() {
        let mut buffer = SpokenPhraseBuffer::new(24);
        buffer.push_chunk("это длинная фраза без пунктуации которая всё ещё продолжается");
        let phrase = buffer.pop_ready().expect("bounded phrase");
        assert!(phrase.chars().count() <= 24);
        assert!(!phrase.ends_with(' '));
    }
}
