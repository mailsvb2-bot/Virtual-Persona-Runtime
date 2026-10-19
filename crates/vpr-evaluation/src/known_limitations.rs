use crate::CheckStatus;

pub(crate) fn parse_known_limitations_review_status(bytes: &[u8]) -> Result<CheckStatus, ()> {
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let mut non_empty = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = non_empty.next().ok_or(())?;
    let first = first.trim_start_matches('﻿');
    let status = match first {
        "RT0-Review-Status: passed" => CheckStatus::Passed,
        "RT0-Review-Status: failed" => CheckStatus::Failed,
        _ => return Err(()),
    };
    if non_empty.next().is_none() {
        return Err(());
    }
    Ok(status)
}

/// Requires exactly one explicit INTERIM marker for an interim voice and prevents
/// a stale INTERIM claim on an ACCEPTED or integrated-speech candidate.
pub(crate) fn voice_marker_matches(bytes: &[u8], expected: Option<&str>) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let markers: Vec<_> = text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("RT0-Voice-Status:"))
        .collect();
    match expected {
        Some("INTERIM") => markers.as_slice() == ["RT0-Voice-Status: INTERIM"],
        _ => markers.is_empty(),
    }
}

#[cfg(test)]
mod interim_voice_tests {
    use super::voice_marker_matches;
    #[test]
    fn interim_requires_exact_marker() {
        assert!(voice_marker_matches(
            b"RT0-Review-Status: passed\nRT0-Voice-Status: INTERIM\nknown limitation\n",
            Some("INTERIM")
        ));
        assert!(!voice_marker_matches(
            b"RT0-Review-Status: passed\nknown limitation",
            Some("INTERIM")
        ));
        assert!(!voice_marker_matches(
            b"RT0-Review-Status: passed\nRT0-Voice-Status: INTERIM\nknown limitation",
            Some("ACCEPTED")
        ));
    }
}
