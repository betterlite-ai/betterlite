use betterlite_core::cost::context::response_time::{ResponseTiming, response_time_ms};
use betterlite_logging::fields::timestamps::Moment;
use serde_json::Value;

use super::super::model::{cost_response, moment};
use super::super::{Call, Failure, TimingInput, failure};

pub(super) fn response_time(call: &Call) -> Result<Value, Failure> {
    let Some(input) = call.response_time.as_ref() else {
        return Err(failure("response_time is required"));
    };
    let moments = moment_values(input.timing.as_ref())?;
    let timing = moments.as_ref().map(|(start, end, now)| ResponseTiming {
        start_time: Some(start),
        end_time: Some(end),
        now,
    });
    let response = cost_response(&input.response);
    let observed =
        response_time_ms(Some(&response), input.total_time, timing.as_ref()).map_err(failure)?;
    Ok(Value::from(observed.normalize().to_string()))
}

fn moment_values(
    timing: Option<&TimingInput>,
) -> Result<Option<(Moment, Moment, Moment)>, Failure> {
    let Some(found) = timing else {
        return Ok(None);
    };
    Ok(Some((
        moment(&found.start_time)?,
        moment(&found.end_time)?,
        moment(&found.now)?,
    )))
}
