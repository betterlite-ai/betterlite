use super::wire::callback::Callback;
use rust_decimal::Decimal;

#[must_use]
pub fn time_to_first_chunk_seconds(kwargs: &Callback) -> Option<Decimal> {
    if !kwargs.optional_params.stream {
        return None;
    }
    kwargs
        .completion_start_time?
        .0
        .checked_sub(kwargs.api_call_start_time?.0)
}

mod tests;
