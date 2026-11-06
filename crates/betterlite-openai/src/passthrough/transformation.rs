use serde::Deserialize as _;
use serde_json::Value;
use url::Url;

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::{Headers, Object};
use types::{
    MAX_COLLECTED_BYTES, PassthroughError, PassthroughLogging, PassthroughMetadata,
    PassthroughSigning, PassthroughTarget, PassthroughUrlRequest, RelayShape, RelayedBody,
    RelayedResponse, query_value_text,
};

mod tests;
pub mod types;

#[must_use]
pub fn model_group_from(litellm_params: &Object) -> String {
    litellm_params
        .get("litellm_metadata")
        .and_then(|metadata| PassthroughMetadata::deserialize(metadata).ok())
        .map_or_else(String::new, |metadata| metadata.model_group)
}

#[must_use]
pub fn strip_leading_model_segment(endpoint: &str, model_names: &[String]) -> String {
    let path = endpoint.trim_start_matches('/');
    for model_name in model_names {
        if model_name.is_empty() {
            continue;
        }
        if path == model_name {
            return String::new();
        }
        if let Some(rest) = path
            .strip_prefix(model_name.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return rest.to_owned();
        }
    }
    path.to_owned()
}

#[must_use]
pub fn replace_path_segment(endpoint: &str, segment: &str, replacement: &str) -> String {
    let pattern = format!("(?<![^/]){}(?![^/:])", fancy_regex::escape(segment));
    let replaced = fancy_regex::Regex::new(&pattern)
        .and_then(|bounded| bounded.try_replacen(endpoint, 0, fancy_regex::NoExpand(replacement)));
    // the escaped pattern always compiles; a backtrack limit leaves the endpoint untouched
    replaced.map_or_else(|_| endpoint.to_owned(), std::borrow::Cow::into_owned)
}

#[must_use]
pub fn relayed_json_object(status_code: u16, body: &[u8]) -> Option<Object> {
    if status_code != 200 {
        return None;
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(object)) => Some(object),
        Ok(_) | Err(_) => None,
    }
}

pub fn relayed_body(body: &[u8]) -> Result<RelayedBody, PassthroughError> {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(object)) => Ok(RelayedBody::Json(object)),
        Ok(_) | Err(_) => String::from_utf8(body.to_vec())
            .map(RelayedBody::Text)
            .map_err(|error| PassthroughError::InvalidUtf8 {
                source: error.utf8_error(),
            }),
    }
}

#[must_use]
pub fn logged_relay_shape<T>(
    shapes: &[RelayShape<'_, T>],
    status_code: u16,
    body: &[u8],
    logging_obj: &mut dyn PassthroughLogging,
    endpoint: &str,
) -> Option<T> {
    let relayed_path = format!("/{}", endpoint.trim_matches('/'));
    let shape = shapes
        .iter()
        .find(|shape| relayed_path.ends_with(shape.path_suffix))?;
    let relayed = relayed_json_object(status_code, body)?;
    let parsed = (shape.parse)(&relayed)?;
    logging_obj.set_call_type(shape.call_type);
    Some(parsed)
}

pub fn convert_raw_bytes_to_lines(chunks: &[&[u8]]) -> Result<Vec<String>, PassthroughError> {
    let joined = chunks.concat();
    let text = String::from_utf8(joined).map_err(|error| PassthroughError::InvalidUtf8 {
        source: error.utf8_error(),
    })?;
    Ok(text
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

pub trait PassthroughStreamCollector {
    type Logged;

    fn add(&mut self, chunk: &[u8]);

    fn build_logged_response(
        &self,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError>;
}

pub struct RawBytesStreamCollector<'config, Config> {
    provider_config: &'config Config,
    model: String,
    custom_llm_provider: String,
    endpoint: String,
    chunks: Vec<Vec<u8>>,
    collected_bytes: usize,
    disabled: bool,
}

impl<Config> PassthroughStreamCollector for RawBytesStreamCollector<'_, Config>
where
    Config: BasePassthroughConfig,
{
    type Logged = Config::Logged;

    fn add(&mut self, chunk: &[u8]) {
        if self.disabled {
            return;
        }
        let Some(total) = self
            .collected_bytes
            .checked_add(chunk.len())
            .filter(|total| *total <= MAX_COLLECTED_BYTES)
        else {
            self.disabled = true;
            self.chunks.clear();
            self.collected_bytes = 0;
            return;
        };
        self.chunks.push(chunk.to_vec());
        self.collected_bytes = total;
    }

    fn build_logged_response(
        &self,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Result<Option<Self::Logged>, PassthroughError> {
        if self.disabled {
            return Ok(None);
        }
        let chunks: Vec<&[u8]> = self.chunks.iter().map(Vec::as_slice).collect();
        let lines = convert_raw_bytes_to_lines(&chunks)?;
        Ok(self.provider_config.handle_collected_chunks(
            &lines,
            logging_obj,
            &PassthroughTarget {
                model: &self.model,
                custom_llm_provider: &self.custom_llm_provider,
                endpoint: &self.endpoint,
            },
        ))
    }
}

pub trait BasePassthroughConfig {
    type Error: From<ProviderError> + From<PassthroughError>;
    type Logged;

    fn is_streaming_request(&self, endpoint: &str, request_data: &Object) -> bool;

    fn format_url(
        &self,
        endpoint: &str,
        base_target_url: &str,
        request_query_params: Option<&Object>,
    ) -> Result<Url, PassthroughError> {
        let base = base_target_url.trim_end_matches('/');
        let native_endpoint = endpoint.trim_start_matches('/');
        let full_url = format!("{base}/{native_endpoint}");
        let mut url = Url::parse(&full_url).map_err(|source| PassthroughError::InvalidUrl {
            url: full_url.clone(),
            source,
        })?;
        if let Some(params) = request_query_params.filter(|params| !params.is_empty()) {
            url.query_pairs_mut()
                .clear()
                .extend_pairs(query_pairs(params)?);
        }
        Ok(url)
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error>;

    fn sign_request(
        &self,
        headers: Headers,
        _context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        Ok((headers, None))
    }

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: Option<&Headers>,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            status_code_is_synthesized: false,
            message: message.to_owned(),
            headers: headers.map(|provided| {
                provided
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            }),
            request: None,
            response: None,
            body: None,
        })
    }

    fn logging_non_streaming_response(
        &self,
        _target: &PassthroughTarget<'_>,
        _response: &RelayedResponse<'_>,
        _request_data: &Object,
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Self::Logged> {
        None
    }

    fn handle_collected_chunks(
        &self,
        _all_chunks: &[String],
        _logging_obj: &mut dyn PassthroughLogging,
        _target: &PassthroughTarget<'_>,
    ) -> Option<Self::Logged> {
        None
    }

    fn create_stream_collector(
        &self,
        target: &PassthroughTarget<'_>,
    ) -> Box<dyn PassthroughStreamCollector<Logged = Self::Logged> + Send + '_>
    where
        Self: Sized + Sync,
    {
        Box::new(RawBytesStreamCollector {
            provider_config: self,
            model: target.model.to_owned(),
            custom_llm_provider: target.custom_llm_provider.to_owned(),
            endpoint: target.endpoint.to_owned(),
            chunks: Vec::new(),
            collected_bytes: 0,
            disabled: false,
        })
    }
}

fn query_pairs(params: &Object) -> Result<Vec<(String, String)>, PassthroughError> {
    params
        .iter()
        .map(|(name, value)| {
            query_value_text(value)
                .map(|text| (name.clone(), text))
                .ok_or_else(|| PassthroughError::QueryValue { name: name.clone() })
        })
        .collect()
}
