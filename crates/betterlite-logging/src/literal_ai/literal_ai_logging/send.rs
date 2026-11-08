use std::error::Error as StdError;
use std::sync::atomic::AtomicUsize;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use url::Url;

use super::graphql;
use super::settings::{GRAPHQL_PATH, Settings, SettingsError};
use super::step::Step;
use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::http_analytics_loggers::delivery::{DELIVERY_DEADLINE, Delivery, PostRequest};

#[derive(Debug, thiserror::Error)]
pub enum SendError<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("LITERAL_API_KEY is not set, so no request is sent")]
    MissingApiKey,
    #[error("the Literal AI GraphQL URL could not be parsed")]
    Url(#[source] url::ParseError),
    #[error("the Literal AI request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("Literal AI answered with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the Literal AI request failed")]
    Transport(#[source] E),
}

#[derive(Serialize)]
struct GraphQlBody {
    query: String,
    variables: IndexMap<String, Json>,
}

// _send_batch and async_send_batch: one POST of the queue's steps as GraphQL query and variables.
pub async fn post_steps<D>(
    delivery: &D,
    settings: &Settings,
    steps: &[Step],
) -> Result<(), SendError<D::Error>>
where
    D: Delivery,
{
    if settings.api_key.is_none() {
        return Err(SendError::MissingApiKey);
    }
    let url = Url::parse(&format!("{}{GRAPHQL_PATH}", settings.api_url)).map_err(SendError::Url)?;
    let headers = settings.headers()?;
    let body = serde_json::to_vec(&GraphQlBody {
        query: graphql::mutation_query(steps),
        variables: graphql::step_variables(steps),
    })
    .map_err(SendError::Encode)?;
    let response = delivery
        .post(PostRequest {
            url,
            headers,
            body,
            deadline: DELIVERY_DEADLINE,
        })
        .await
        .map_err(SendError::Transport)?;
    if response.status < 300 {
        tracing::debug!("Batch of {} runs successfully created", steps.len());
        return Ok(());
    }
    Err(SendError::Status {
        status: response.status,
        body: String::from_utf8(response.body).unwrap_or_default(),
    })
}

// flush_queue's async_send_batch: the drain already happened, and its outcome never reaches the queue.
pub struct BatchSender<'logger, D> {
    pub settings: &'logger Settings,
    pub delivery: &'logger D,
}

impl<D> BatchDelivery<Step> for BatchSender<'_, D>
where
    D: Delivery,
{
    async fn deliver(
        &self,
        batch: &[Step],
        _settled: &AtomicUsize,
    ) -> Result<Vec<Step>, BatchCancelled<Step>> {
        if let Err(error) = post_steps(self.delivery, self.settings, batch).await {
            if let SendError::Status { status, body } = &error {
                tracing::error!("Literal AI HTTP Error: {status} - {body}");
            } else {
                tracing::error!("Literal AI Layer Error: {error}");
            }
        }
        Ok(Vec::new())
    }
}
