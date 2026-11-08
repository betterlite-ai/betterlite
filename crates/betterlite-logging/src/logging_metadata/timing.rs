use rust_decimal::Decimal;

use crate::fields::timestamps::TimestampError;

pub fn get_response_time(
    start_time: Decimal,
    end_time: Decimal,
    completion_start_time: Decimal,
    stream: bool,
) -> Result<Decimal, TimestampError> {
    let difference = if stream {
        completion_start_time.checked_sub(start_time)
    } else {
        end_time.checked_sub(start_time)
    };
    difference.ok_or(TimestampError::ResponseTimeOverflow)
}

mod tests;
