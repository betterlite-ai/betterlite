use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;

use super::transport::{ArgillaMethod, ArgillaRequest, Transport};

#[derive(Clone, Debug)]
pub struct ArgillaCredentials {
    pub api_key: SecretString,
    pub base_url: String,
    // holds the resolved dataset id once the logger connected
    pub dataset_name: String,
}

#[derive(Debug, Deserialize)]
struct DatasetPage {
    #[serde(default)]
    items: Option<Json>,
}

#[derive(Debug, thiserror::Error)]
pub enum DatasetError<E> {
    #[error("the Argilla dataset lookup failed")]
    Transport {
        #[source]
        source: E,
    },
    #[error("the Argilla dataset response is not JSON")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the first Argilla dataset has no string id")]
    Id,
}

pub async fn resolve_dataset<T>(
    transport: &T,
    credentials: &ArgillaCredentials,
) -> Result<String, DatasetError<T::Error>>
where
    T: Transport,
{
    let request = ArgillaRequest {
        method: ArgillaMethod::Get,
        url: format!(
            "{}/api/v1/me/datasets?name={}",
            credentials.base_url, credentials.dataset_name
        ),
        api_key: credentials.api_key.clone(),
        body: None,
        timeout: None,
    };
    let response = transport
        .send(request)
        .await
        .map_err(|source| DatasetError::Transport { source })?;
    let page: DatasetPage =
        serde_json::from_str(&response.body).map_err(|source| DatasetError::Body { source })?;
    let Some(Json::List(items)) = page.items.as_ref() else {
        return Ok(credentials.dataset_name.clone());
    };
    let Some(first) = items.first() else {
        return Ok(credentials.dataset_name.clone());
    };
    let Json::Object(fields) = first else {
        return Err(DatasetError::Id);
    };
    match fields.get("id") {
        Some(Json::Text(id)) => Ok(id.clone()),
        _ => Err(DatasetError::Id),
    }
}
