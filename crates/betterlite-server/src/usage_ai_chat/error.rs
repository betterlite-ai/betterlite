use std::error::Error;

use betterlite_db::daily_activity_contracts::identifiers::DailyActivityError;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

use super::model::ModelError;
use crate::daily_activity::pages::error::PagesError;

#[derive(Debug, Error)]
pub enum UsageChatError {
    #[error("Missing required start_date or end_date from tool arguments")]
    MissingDates,
    #[error(
        "Non-admin caller has user_id=None; refusing to issue an unscoped query. Endpoint-level guard missing."
    )]
    UnscopedQuery,
    #[error("the tool arguments are not a JSON object of strings: {source}")]
    InvalidToolArguments {
        #[source]
        source: serde_json::Error,
    },
    #[error("{}", CommonProxyErrors::DbNotConnectedError.as_str())]
    DatabaseNotConnected,
    #[error("the analytics query failed with status {status_code}: {message}")]
    Analytics { status_code: u16, message: String },
    #[error(transparent)]
    Pages(#[from] PagesError),
    #[error(transparent)]
    Scope(#[from] DailyActivityError),
    #[error("the usage totals overflowed")]
    Overflow,
    #[error("the entity {entity} has an alias that is not text")]
    EntityAlias { entity: String },
}

#[derive(Debug, Error)]
pub enum ChatError<Fault: Error + 'static> {
    #[error(transparent)]
    Usage(#[from] UsageChatError),
    #[error(transparent)]
    Model(#[from] ModelError<Fault>),
    #[error("the client left the response stream")]
    ClientDisconnected(#[from] SendError<String>),
}
