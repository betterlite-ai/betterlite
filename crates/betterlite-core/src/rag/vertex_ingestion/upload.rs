use std::time::Duration;

use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;
use thiserror::Error;
use tracing::{debug, error, warn};

use super::API_VERSION;
use super::VertexIngestion;
use super::error::VertexIngestionError;
use super::json::{field, object, response_text};
use super::ports::MultipartRequest;

const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const OCTET_STREAM: &str = "application/octet-stream";

impl VertexIngestion {
    pub(super) async fn upload_file(
        &self,
        corpus_id: &str,
        filename: &str,
        content: &[u8],
        content_type: Option<&str>,
    ) -> Result<String, VertexIngestionError> {
        let token = self.bearer_token(self.project.as_deref()).await?;
        let base_url = self.base_url()?;
        let url = format!("{base_url}/upload/{API_VERSION}/{corpus_id}/ragFiles:upload");
        let encoded =
            serde_json::to_string(&self.upload_metadata(filename)?).map_err(|source| {
                VertexIngestionError::Write {
                    step: "upload metadata",
                    source,
                }
            })?;
        debug!("Uploading file to RAG corpus: {url}");
        debug!("Metadata: {encoded}");
        let reply = self
            .ports
            .http
            .post_multipart(MultipartRequest {
                url: &url,
                token: &token,
                metadata: encoded.as_bytes(),
                filename,
                file: content,
                file_content_type: content_type
                    .filter(|value| !value.is_empty())
                    .unwrap_or(OCTET_STREAM),
                timeout: UPLOAD_TIMEOUT,
            })
            .await?;
        if !matches!(reply.status, 200 | 201) {
            let failure = VertexIngestionError::Upload {
                body: response_text(&reply.body),
            };
            error!("{failure}");
            return Err(failure);
        }
        let file_id = match file_id(&reply.body) {
            Ok(file_id) => file_id,
            Err(reason) => {
                warn!("Could not parse upload response: {reason}");
                "uploaded".to_owned()
            }
        };
        debug!("Upload complete. File ID: {file_id}");
        Ok(file_id)
    }

    fn upload_metadata(&self, filename: &str) -> Result<Json, VertexIngestionError> {
        let mut rag_file = Extra::new();
        rag_file.insert("display_name".to_owned(), Json::Text(filename.to_owned()));
        if let Some(description) = self
            .options
            .file_description
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            rag_file.insert("description".to_owned(), Json::Text(description.to_owned()));
        }
        let mut metadata = Extra::new();
        metadata.insert("rag_file".to_owned(), Json::Object(rag_file));
        if let Some(chunking) = self.chunking_config()? {
            metadata.insert("upload_rag_file_config".to_owned(), chunking);
        }
        Ok(Json::Object(metadata))
    }

    fn chunking_config(&self) -> Result<Option<Json>, VertexIngestionError> {
        let strategy =
            serde_json::from_str::<Json>(self.state.chunking_strategy.get()).map_err(|source| {
                VertexIngestionError::Decode {
                    step: "chunking strategy",
                    source,
                }
            })?;
        let chunk_size = nonzero_integer(&strategy, "chunk_size")?;
        let chunk_overlap = nonzero_integer(&strategy, "chunk_overlap")?;
        if chunk_size.is_none() && chunk_overlap.is_none() {
            return Ok(None);
        }
        let mut fixed = Extra::new();
        if let Some(size) = chunk_size {
            fixed.insert("chunk_size".to_owned(), Json::Number(Number::from(size)));
        }
        if let Some(overlap) = chunk_overlap {
            fixed.insert(
                "chunk_overlap".to_owned(),
                Json::Number(Number::from(overlap)),
            );
        }
        Ok(Some(object([(
            "rag_file_transformation_config",
            object([(
                "rag_file_chunking_config",
                object([("fixed_length_chunking", Json::Object(fixed))]),
            )]),
        )])))
    }
}

// a zero is how LiteLLM's truthiness test drops a size, so it counts as not sent
fn nonzero_integer(
    strategy: &Json,
    name: &'static str,
) -> Result<Option<i64>, VertexIngestionError> {
    match field(strategy, name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => match number.as_i64() {
            Some(0) => Ok(None),
            Some(value) => Ok(Some(value)),
            None => Err(VertexIngestionError::IntegerField { field: name }),
        },
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(VertexIngestionError::IntegerField { field: name })
        }
    }
}

fn named(fields: &Extra) -> Option<String> {
    match fields.get("name") {
        Some(Json::Text(name)) => Some(name.clone()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn file_id(body: &[u8]) -> Result<String, UploadResponseError> {
    let parsed: Json = match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(source) => return Err(UploadResponseError::Decode { source }),
    };
    let Json::Object(fields) = parsed else {
        return Err(UploadResponseError::NotObject);
    };
    match fields.get("ragFile") {
        None => {}
        Some(Json::Object(rag_file)) => {
            if let Some(name) = named(rag_file).filter(|name| !name.is_empty()) {
                return Ok(name);
            }
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(UploadResponseError::RagFileNotObject);
        }
    }
    Ok(named(&fields).unwrap_or_default())
}

// the upload still succeeds when the reply cannot be read, so these reasons only reach the log
#[derive(Debug, Error)]
enum UploadResponseError {
    #[error("the response body is not valid JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response body is not a JSON object")]
    NotObject,
    #[error("`ragFile` is not an object")]
    RagFileNotObject,
}
