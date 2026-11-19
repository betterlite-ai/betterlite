use betterlite_db::gateway_request_tracking::GATEWAY_REQUEST_DATE_FORMAT;
use betterlite_db::gateway_request_tracking::GatewayRequestClock;
use chrono::Days;

const DEFAULT_LOOKBACK_DAYS: u64 = 30;
const START_DATE: &str = "start_date";
const END_DATE: &str = "end_date";

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct ActivityQuery {
    start_date: Option<String>,
    end_date: Option<String>,
}

impl ActivityQuery {
    pub(super) fn decode(raw: &str) -> Self {
        let mut start_date = None;
        let mut end_date = None;
        for (name, value) in form_urlencoded::parse(raw.as_bytes()) {
            match name.as_ref() {
                START_DATE => start_date = Some(value.into_owned()),
                END_DATE => end_date = Some(value.into_owned()),
                _ => {}
            }
        }
        Self {
            start_date,
            end_date,
        }
    }

    pub(super) fn range(&self, clock: &dyn GatewayRequestClock) -> (String, String) {
        let end = clock.utc_now();
        let start = end
            .checked_sub_days(Days::new(DEFAULT_LOOKBACK_DAYS))
            .unwrap_or(end);
        (
            self.start_date
                .clone()
                .unwrap_or_else(|| start.format(GATEWAY_REQUEST_DATE_FORMAT).to_string()),
            self.end_date
                .clone()
                .unwrap_or_else(|| end.format(GATEWAY_REQUEST_DATE_FORMAT).to_string()),
        )
    }
}
