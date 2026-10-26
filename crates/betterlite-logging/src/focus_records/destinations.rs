use chrono::{DateTime, Utc};
use std::future::Future;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusTimeWindow {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub frequency: String,
}

pub trait FocusDestination {
    type Error;

    fn deliver<'a>(
        &'a self,
        content: &'a [u8],
        time_window: &'a FocusTimeWindow,
        filename: &'a str,
    ) -> impl Future<Output = Result<(), Self::Error>> + 'a;
}
