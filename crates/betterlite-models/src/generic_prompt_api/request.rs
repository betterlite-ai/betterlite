use std::error::Error;
use std::future::Future;
use std::time::Duration;

use indexmap::IndexMap;
use secrecy::SecretString;

use crate::decisions::Json;
use crate::generic_prompt_api::error::PromptFetchError;

#[derive(Clone, Debug)]
pub struct PromptApiRequest {
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(&'static str, SecretString)>,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct PromptApiResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait PromptApiTransport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get(&self, request: &PromptApiRequest) -> Result<PromptApiResponse, Self::Error>;

    fn get_async(
        &self,
        request: &PromptApiRequest,
    ) -> impl Future<Output = Result<PromptApiResponse, Self::Error>> + Send;
}

fn scalar_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => Some(String::new()),
        Json::Bool(flag) => Some(flag.to_string()),
        Json::Number(number) => Some(number.to_string()),
        Json::Text(text) => Some(text.clone()),
        Json::List(_) | Json::Object(_) => None,
    }
}

// httpx sends None as an empty value and a list as one pair per item
pub fn query_pairs<TransportError>(
    prompt_id: Option<&str>,
    provider_specific: Option<&IndexMap<String, Json>>,
) -> Result<Vec<(String, String)>, PromptFetchError<TransportError>> {
    let prompt_id_value = prompt_id.map_or(Json::Null, |id| Json::Text(id.to_owned()));
    let mut merged: IndexMap<&str, &Json> = IndexMap::new();
    merged.insert("prompt_id", &prompt_id_value);
    for (key, value) in provider_specific.into_iter().flatten() {
        merged.insert(key, value);
    }
    let mut pairs = Vec::new();
    for (key, value) in merged {
        let items = match value {
            Json::List(items) => items.iter().collect(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
                vec![value]
            }
        };
        for item in items {
            let text = scalar_text(item).ok_or_else(|| PromptFetchError::QueryValue {
                key: key.to_owned(),
            })?;
            pairs.push((key.to_owned(), text));
        }
    }
    Ok(pairs)
}
