use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::lemonade_model_metadata::{
    DeploymentCredentials, LemonadeCredentials, ModelInfoClient, ModelInfoRequest, auth_header,
    context_window, get_model_info, positive_int, provider_specific_entry, resolve_origin,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{Input, ModelInfoQuery, OriginQuery, Reply};
use crate::helpers::{HelperError, HelperResult};

pub(crate) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Origin { queries } => Ok(json!(
            queries.iter().map(origin_answer).collect::<Vec<Value>>()
        )),
        Input::PositiveInt {
            values,
            model_infos,
        } => Ok(json!({
            "values": values.iter().map(positive_int).collect::<Vec<_>>(),
            "context_windows": model_infos.iter().map(context_window).collect::<Vec<_>>(),
        })),
        Input::ProviderEntry { model_infos } => Ok(json!({
            "entries": model_infos.iter().map(provider_specific_entry).collect::<Vec<_>>(),
            "model_infos": model_infos,
        })),
        Input::ModelInfo { queries } => {
            let answers = queries
                .iter()
                .map(model_info_answer)
                .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
            Ok(json!(answers))
        }
    }
}

fn origin_answer(query: &OriginQuery) -> Value {
    let settings = credentials(&query.environment, &query.global_keys);
    let explicit_key = query.api_key.as_deref().map(SecretString::from);
    let origin = resolve_origin(
        &settings,
        DeploymentCredentials {
            api_base: query.api_base.as_deref(),
            api_key: explicit_key.as_ref(),
        },
    );
    let headers = match auth_header(&origin.api_key) {
        Some((name, value)) => {
            BTreeMap::from([(name.to_owned(), value.expose_secret().to_owned())])
        }
        None => BTreeMap::new(),
    };
    json!({
        "api_base": origin.api_base,
        "api_key": origin.api_key.expose_secret(),
        "headers": headers,
    })
}

fn model_info_answer(query: &ModelInfoQuery) -> Result<Value, Box<HelperError>> {
    let client = FixtureClient {
        reply: query.reply.as_ref(),
        seen: Mutex::default(),
    };
    let settings = credentials(&query.environment, &query.global_keys);
    let explicit_key = query.api_key.as_deref().map(SecretString::from);
    let deployment = DeploymentCredentials {
        api_base: query.api_base.as_deref(),
        api_key: explicit_key.as_ref(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| Box::new(failure(error.to_string())))?;
    let timeout = Duration::from_secs(query.timeout_seconds);
    let info = runtime
        .block_on(get_model_info(
            &client,
            &settings,
            &query.model,
            deployment,
            timeout,
        ))
        .map_err(|error| Box::new(failure(error.to_string())))?;
    let requests = client.requests();
    let request = requests
        .first()
        .ok_or_else(|| Box::new(failure("the fixture transport saw no request".to_owned())))?;
    Ok(json!({
        "info": serde_json::to_value(info).map_err(|error| Box::new(failure(error.to_string())))?,
        "url": &request.url,
        "authorization": authorization(&request.headers),
    }))
}

fn credentials(
    environment: &BTreeMap<String, String>,
    global_keys: &BTreeMap<String, String>,
) -> LemonadeCredentials {
    LemonadeCredentials {
        api_base: environment.get("LEMONADE_API_BASE").cloned(),
        api_key: environment
            .get("LEMONADE_API_KEY")
            .cloned()
            .map(SecretString::from),
        lemonade_key: global_keys
            .get("lemonade_key")
            .cloned()
            .map(SecretString::from),
    }
}

fn authorization<'headers>(
    headers: &'headers [(&'static str, SecretString)],
) -> Option<&'headers str> {
    headers
        .iter()
        .find_map(|(name, value)| (*name == "Authorization").then_some(value.expose_secret()))
}

struct FixtureClient<'reply> {
    reply: Option<&'reply Reply>,
    seen: Mutex<Vec<ModelInfoRequest>>,
}

impl FixtureClient<'_> {
    fn requests(&self) -> Vec<ModelInfoRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fixture transport failed")]
struct FixtureError;

impl ModelInfoClient for FixtureClient<'_> {
    type Error = FixtureError;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        ready(scripted_reply(self.reply))
    }
}

fn scripted_reply(reply: Option<&Reply>) -> Result<FetchResponse, FixtureError> {
    let Some(scripted) = reply else {
        return Err(FixtureError);
    };
    let body = match (&scripted.raw, &scripted.body) {
        (Some(raw), _) => raw.as_bytes().to_vec(),
        (None, Some(body)) => serde_json::to_vec(body).map_err(|_| FixtureError)?,
        (None, None) => Vec::new(),
    };
    Ok(FetchResponse {
        status: scripted.status,
        headers: BTreeMap::new(),
        body,
    })
}

fn failure(message: String) -> HelperError {
    HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
