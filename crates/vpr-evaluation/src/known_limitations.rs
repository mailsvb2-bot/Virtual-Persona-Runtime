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
