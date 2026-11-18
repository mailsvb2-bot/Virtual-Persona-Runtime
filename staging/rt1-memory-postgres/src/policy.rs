//! Pure, deterministic, cheap memory policy. No paid LLM calls or GPU work.
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub key: String,
    pub content: String,
    pub importance: u8,
    pub category: &'static str,
    pub digest: String,
    pub retention_days: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconcile {
    Same,
    Replace,
    Distinct,
    NeedsConfirmation,
}

pub fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_lowercase()
}

pub fn fingerprint(text: &str) -> String {
    Sha256::digest(normalize(text).as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn significance(text: &str) -> u8 {
    let s = normalize(text);
    if s.contains("всегда")
        || s.contains("аллерг")
        || s.contains("never")
        || s.contains("не предлагай")
    {
        5
    } else if s.contains("важно") || s.contains("предпочита") || s.contains("люблю")
    {
        4
    } else if s.contains("работаю") || s.contains("живу") || s.contains("предпоч")
    {
        3
    } else {
        2
    }
}

pub fn category(text: &str) -> &'static str {
    let s = normalize(text);
    if s.contains("работаю") || s.contains("уволил") || s.contains("работа") {
        "work"
    } else if s.contains("живу") || s.contains("переехал") {
        "residence"
    } else if s.contains("люблю") || s.contains("нравится") || s.contains("предпоч")
    {
        "preference"
    } else {
        "explicit_note"
    }
}

pub fn candidate(key: &str, content: &str, days: i32) -> Option<MemoryCandidate> {
    if key.is_empty()
        || key.len() > 96
        || content.trim().is_empty()
        || content.len() > 2048
        || !(1..=3650).contains(&days)
    {
        return None;
    }
    Some(MemoryCandidate {
        key: key.to_owned(),
        content: content.trim().to_owned(),
        importance: significance(content),
        category: category(content),
        digest: fingerprint(content),
        retention_days: days,
    })
}

pub fn reconcile(existing_key: &str, existing: &str, next_key: &str, next: &str) -> Reconcile {
    if fingerprint(existing) == fingerprint(next) {
        return Reconcile::Same;
    }
    if existing_key == next_key {
        return Reconcile::Replace;
    }
    // Heuristics must NEVER silently overwrite someone else's explicit claim.
    if category(existing) == category(next) && category(existing) != "explicit_note" {
        return Reconcile::NeedsConfirmation;
    }
    Reconcile::Distinct
}

pub fn useful_terms(s: &str) -> Vec<String> {
    let stop: HashSet<&str> = [
        "я",
        "ты",
        "мне",
        "меня",
        "что",
        "где",
        "как",
        "кто",
        "а",
        "и",
        "на",
        "в",
        "не",
        "это",
        "про",
        "когда",
        "мы",
        "вы",
        "обо",
        "мной",
    ]
    .into_iter()
    .collect();
    let mut result = Vec::new();
    for term in s.split(|c: char| !c.is_alphanumeric()) {
        let low = term.to_lowercase();
        if low.chars().count() >= 3 && !stop.contains(low.as_str()) && !result.contains(&low) {
            result.push(low);
        }
        if result.len() >= 12 {
            break;
        }
    }
    result
}

pub fn context_budget(facts: &[super::Fact], max_bytes: usize, max_tokens: usize) -> String {
    let mut out = String::new();
    // Conservative Unicode-safe token approximation, not a tokenizer billing estimate.
    let mut tokens = 0usize;
    for fact in facts.iter().take(8) {
        let Ok(quoted) = serde_json::to_string(&fact.content) else {
            continue;
        };
        let line = format!("- key={} value={}\n", fact.key, quoted);
        let cost = line.chars().count() / 3 + 1;
        if out.len() + line.len() > max_bytes || tokens + cost > max_tokens {
            break;
        }
        out.push_str(&line);
        tokens += cost;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedupe_and_conflicts() {
        assert_eq!(
            reconcile("a", "Люблю чай", "b", "  люблю  чай "),
            Reconcile::Same
        );
        assert_eq!(
            reconcile("work", "работаю врачом", "work", "уволился"),
            Reconcile::Replace
        );
        assert_eq!(
            reconcile("one", "работаю врачом", "two", "уволился"),
            Reconcile::NeedsConfirmation
        );
    }
    #[test]
    fn no_giant_context() {
        let f = super::super::Fact {
            key: "a".into(),
            content: "я".repeat(1000),
            revision: 1,
        };
        let v = context_budget(&[f], 100, 30);
        assert!(v.len() <= 100);
    }
}
