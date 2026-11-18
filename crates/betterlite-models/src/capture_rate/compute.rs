use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;

use super::report::{CaptureRateDay, CaptureRateReport};
use super::settings::SpendCaptureProvider;

#[derive(Debug, thiserror::Error)]
pub enum CaptureRateError {
    #[error("summing the captured spend overflowed the money range")]
    CapturedOverflow,
    #[error("summing the provider bill overflowed the money range")]
    ProviderOverflow,
    #[error("dividing the captured spend by the provider bill overflowed the money range")]
    RatioOverflow,
    #[error("the capture-rate window runs past the last representable day")]
    DayOverflow,
}

pub struct CaptureRateRequest<'amounts> {
    pub provider: SpendCaptureProvider,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub captured_by_day: &'amounts BTreeMap<String, Decimal>,
    pub billed_by_day: &'amounts BTreeMap<String, Decimal>,
    pub threshold: Decimal,
}

pub fn compute_capture_rate(
    request: &CaptureRateRequest<'_>,
) -> Result<CaptureRateReport, CaptureRateError> {
    let span = request
        .end_date
        .signed_duration_since(request.start_date)
        .num_days();
    let mut days = Vec::new();
    let mut captured_total = Decimal::ZERO;
    let mut billed_total = Decimal::ZERO;
    for offset in 0..=span {
        let back = u64::try_from(offset).map_err(|_error| CaptureRateError::DayOverflow)?;
        let day = request
            .start_date
            .checked_add_days(Days::new(back))
            .ok_or(CaptureRateError::DayOverflow)?;
        let date = day.format("%Y-%m-%d").to_string();
        let captured = request
            .captured_by_day
            .get(&date)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let billed = request
            .billed_by_day
            .get(&date)
            .copied()
            .unwrap_or(Decimal::ZERO);
        captured_total = captured_total
            .checked_add(captured)
            .ok_or(CaptureRateError::CapturedOverflow)?;
        billed_total = billed_total
            .checked_add(billed)
            .ok_or(CaptureRateError::ProviderOverflow)?;
        days.push(CaptureRateDay {
            date,
            captured_spend: captured,
            provider_spend: billed,
            capture_rate: ratio(captured, billed)?,
        });
    }
    let capture_rate = ratio(captured_total, billed_total)?;
    Ok(CaptureRateReport {
        provider: request.provider,
        start_date: request.start_date.format("%Y-%m-%d").to_string(),
        end_date: request.end_date.format("%Y-%m-%d").to_string(),
        captured_spend: captured_total,
        provider_spend: billed_total,
        capture_rate,
        threshold: request.threshold,
        below_threshold: capture_rate.is_some_and(|rate| rate < request.threshold),
        days,
    })
}

fn ratio(captured: Decimal, billed: Decimal) -> Result<Option<Decimal>, CaptureRateError> {
    if billed <= Decimal::ZERO {
        return Ok(None);
    }
    captured
        .checked_div(billed)
        .map(Some)
        .ok_or(CaptureRateError::RatioOverflow)
}
