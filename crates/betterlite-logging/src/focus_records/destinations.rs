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

    fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'delivery;
}

mod tests;
