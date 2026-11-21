use std::time::Duration;

use betterlite_models::decisions::{Extra, Json};
use secrecy::SecretString;
use tokio::time::sleep;
use tracing::{debug, error};

use crate::api::vertex_location::validate_vertex_project;

use super::API_VERSION;
use super::VertexIngestion;
use super::error::VertexIngestionError;
use super::json::{object, response_text};
use super::ports::{GetRequest, JsonRequest};

const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_ATTEMPTS: usize = 30;
const RETRY_DELAY: Duration = Duration::from_secs(2);

pub(super) async fn create_corpus(
    ingestion: &VertexIngestion,
    project: &str,
    display_name: &str,
) -> Result<String, VertexIngestionError> {
    let checked_project = validate_vertex_project(project)?;
    let token = ingestion.bearer_token(Some(checked_project)).await?;
    let base_url = ingestion.base_url()?;
    let url = format!(
        "{base_url}/{API_VERSION}/projects/{checked_project}/locations/{}/ragCorpora",
        ingestion.location
    );
    debug!("Creating RAG corpus: {url}");
    let operation = post_corpus(ingestion, &url, &token, display_name).await?;
    let corpus = corpus_from_operation(ingestion, &operation, &token).await?;
    debug!("Created RAG corpus: {corpus}");
    Ok(corpus)
}

async fn post_corpus(
    ingestion: &VertexIngestion,
    url: &str,
    token: &SecretString,
    display_name: &str,
) -> Result<Json, VertexIngestionError> {
    let encoded =
        serde_json::to_string(&corpus_body(ingestion, display_name)?).map_err(|source| {
            VertexIngestionError::Write {
                step: "create corpus",
                source,
            }
        })?;
    debug!("Request body: {encoded}");
    let reply = ingestion
        .ports
        .http
        .post_json(JsonRequest {
            url,
            token,
            body: encoded.as_bytes(),
            timeout: CREATE_TIMEOUT,
        })
        .await?;
    if !matches!(reply.status, 200 | 201) {
        let failure = VertexIngestionError::CreateCorpus {
            body: response_text(&reply.body),
        };
        error!("{failure}");
        return Err(failure);
    }
    debug!("Create corpus response: {}", response_text(&reply.body));
    serde_json::from_slice(&reply.body).map_err(|source| VertexIngestionError::Decode {
        step: "create corpus",
        source,
    })
}

async fn corpus_from_operation(
    ingestion: &VertexIngestion,
    operation: &Json,
    token: &SecretString,
) -> Result<String, VertexIngestionError> {
    let Json::Object(fields) = operation else {
        return Err(VertexIngestionError::Shape {
            step: "create corpus",
        });
    };
    if is_done(fields)? {
        return response_name(fields);
    }
    let operation_name = match fields.get("name") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(name)) => name.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(VertexIngestionError::TextField { field: "name" });
        }
    };
    debug!("Polling operation: {operation_name}");
    poll_operation(ingestion, &operation_name, token).await
}

fn corpus_body(
    ingestion: &VertexIngestion,
    display_name: &str,
) -> Result<Json, VertexIngestionError> {
    let mut body = Extra::new();
    body.insert(
        "displayName".to_owned(),
        Json::Text(display_name.to_owned()),
    );
    if let Some(description) = ingestion
        .options
        .description
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        body.insert("description".to_owned(), Json::Text(description.to_owned()));
    }
    if let Some(vector_db) = vector_db_section(ingestion)? {
        body.insert("vectorDbConfig".to_owned(), vector_db);
    }
    Ok(Json::Object(body))
}

fn vector_db_section(ingestion: &VertexIngestion) -> Result<Option<Json>, VertexIngestionError> {
    let mut section = match &ingestion.options.vector_db_config {
        None => Extra::new(),
        Some(config) => {
            let text = serde_json::to_string(config)
                .map_err(|source| VertexIngestionError::Value { source })?;
            match serde_json::from_str::<Json>(&text)
                .map_err(|source| VertexIngestionError::Value { source })?
            {
                Json::Object(fields) => fields,
                Json::Null => Extra::new(),
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    return Err(VertexIngestionError::VectorDbConfig);
                }
            }
        }
    };
    if let Some(model) = ingestion
        .options
        .embedding_model
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        let endpoint = object([("endpoint", Json::Text(model.to_owned()))]);
        let prediction = object([("vertexPredictionEndpoint", endpoint)]);
        section.insert("ragEmbeddingModelConfig".to_owned(), prediction);
    }
    if section.is_empty() {
        return Ok(None);
    }
    Ok(Some(Json::Object(section)))
}

async fn poll_operation(
    ingestion: &VertexIngestion,
    operation_name: &str,
    token: &SecretString,
) -> Result<String, VertexIngestionError> {
    let base_url = ingestion.base_url()?;
    let url = format!("{base_url}/{API_VERSION}/{operation_name}");
    for attempt in 1..=POLL_ATTEMPTS {
        let reply = ingestion
            .ports
            .http
            .get(GetRequest {
                url: &url,
                token,
                timeout: POLL_TIMEOUT,
            })
            .await?;
        if reply.status != 200 {
            let failure = VertexIngestionError::Poll {
                body: response_text(&reply.body),
            };
            error!("{failure}");
            return Err(failure);
        }
        let operation: Json =
            serde_json::from_slice(&reply.body).map_err(|source| VertexIngestionError::Decode {
                step: "poll",
                source,
            })?;
        if let Some(corpus) = done_corpus(&operation)? {
            return Ok(corpus);
        }
        debug!("Operation not done yet, attempt {attempt}/{POLL_ATTEMPTS}");
        sleep(RETRY_DELAY).await;
    }
    Err(VertexIngestionError::OperationTimeout)
}

fn done_corpus(operation: &Json) -> Result<Option<String>, VertexIngestionError> {
    let Json::Object(fields) = operation else {
        return Err(VertexIngestionError::Shape { step: "poll" });
    };
    if !is_done(fields)? {
        return Ok(None);
    }
    if let Some(error_value) = fields.get("error") {
        let text =
            serde_json::to_string(error_value).map_err(|source| VertexIngestionError::Write {
                step: "poll",
                source,
            })?;
        return Err(VertexIngestionError::OperationFailed { error: text });
    }
    let name = response_name(fields)?;
    if name.is_empty() {
        let operation_text =
            serde_json::to_string(operation).map_err(|source| VertexIngestionError::Write {
                step: "poll",
                source,
            })?;
        return Err(VertexIngestionError::MissingCorpusName {
            operation: operation_text,
        });
    }
    Ok(Some(name))
}

fn is_done(fields: &Extra) -> Result<bool, VertexIngestionError> {
    let Some(flag) = fields.get("done") else {
        return Ok(false);
    };
    if matches!(flag, Json::Null) {
        return Ok(false);
    }
    if let Json::Bool(done) = flag {
        return Ok(*done);
    }
    Err(VertexIngestionError::BooleanField { field: "done" })
}

fn response_name(fields: &Extra) -> Result<String, VertexIngestionError> {
    let response = match fields.get("response") {
        None | Some(Json::Null) => return Ok(String::new()),
        Some(Json::Object(response)) => response,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VertexIngestionError::TextField { field: "response" });
        }
    };
    match response.get("name") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(name)) => Ok(name.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VertexIngestionError::TextField {
                field: "response.name",
            })
        }
    }
}
