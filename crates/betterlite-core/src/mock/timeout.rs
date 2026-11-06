use std::time::Duration;

use betterlite_config::sections::mock_response::failure::MockError;
use serde::Deserialize;
use tokio::time::sleep;

// LiteLLM's _sleep_for_timeout accepts a number, a numeric string or an httpx.Timeout
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum TimeoutValue {
    Seconds(f64),
    Text(String),
}

pub fn timeout_duration(value: &TimeoutValue) -> Result<Duration, MockError> {
    let seconds = match value {
        TimeoutValue::Seconds(seconds) => *seconds,
        TimeoutValue::Text(text) => text
            .parse::<f64>()
            .map_err(|_source| MockError::WrongType { field: "timeout" })?,
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|_source| MockError::WrongType { field: "timeout" })
}

pub fn delay_duration(mock_delay: Option<f64>) -> Result<Duration, MockError> {
    mock_delay.map_or(Ok(Duration::ZERO), |seconds| {
        Duration::try_from_secs_f64(seconds).map_err(|_source| MockError::WrongType {
            field: "mock_delay",
        })
    })
}

// _handle_mock_timeout sleeps for the deployment timeout and raises only when both inputs are set
pub async fn raise_on_timeout(
    mock_timeout: Option<bool>,
    timeout: Option<&TimeoutValue>,
) -> Result<(), MockError> {
    match (mock_timeout, timeout) {
        (Some(true), Some(value)) => {
            sleep(timeout_duration(value)?).await;
            Err(MockError::Timeout)
        }
        (Some(true), None) | (Some(false) | None, _) => Ok(()),
    }
}

pub async fn sleep_delay(mock_delay: Option<f64>) -> Result<(), MockError> {
    let wait = delay_duration(mock_delay)?;
    if !wait.is_zero() {
        sleep(wait).await;
    }
    Ok(())
}

mod tests;
