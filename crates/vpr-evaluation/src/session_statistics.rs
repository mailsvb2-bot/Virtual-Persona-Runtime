use crate::{LabSessionAggregateError, LatencyDistributionMillis};

pub(crate) fn add_cost(
    total: &mut Option<u64>,
    value: Option<u64>,
) -> Result<(), LabSessionAggregateError> {
    let (Some(current), Some(value)) = (*total, value) else {
        *total = None;
        return Ok(());
    };
    *total = Some(
        current
            .checked_add(value)
            .ok_or(LabSessionAggregateError::Overflow)?,
    );
    Ok(())
}

pub(crate) fn distribution(
    mut values: Vec<u64>,
) -> Result<Option<LatencyDistributionMillis>, LabSessionAggregateError> {
    if values.is_empty() {
        return Ok(None);
    }
    values.sort_unstable();
    let samples = u32::try_from(values.len()).map_err(|_| LabSessionAggregateError::Overflow)?;
    Ok(Some(LatencyDistributionMillis {
        samples,
        p50: nearest_rank(&values, 50),
        p95: nearest_rank(&values, 95),
    }))
}

fn nearest_rank(values: &[u64], percentile: usize) -> u64 {
    let rank = values.len().saturating_mul(percentile).div_ceil(100).max(1);
    values[rank - 1]
}
