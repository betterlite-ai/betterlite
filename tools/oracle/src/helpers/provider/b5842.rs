mod calls;
mod input;
mod raised;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use tokio::runtime::Builder;

use self::input::{Family, Input};

struct NoTransport;

impl WatsonxTransport for NoTransport {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

fn environment(values: &BTreeMap<String, String>) -> WatsonxEnvironment {
    let secret = |name: &str| {
        values
            .get(name)
            .map(|value| SecretString::from(value.clone()))
    };
    let text = |name: &str| values.get(name).cloned();
    WatsonxEnvironment {
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        ..WatsonxEnvironment::default()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let mut builder = Builder::new_current_thread();
        let runtime = builder.enable_time().build().or_config_error()?;
        let auth = WatsonxAuth::new(environment(&input.env), NoTransport, Arc::new(|| 0.0_f64))
            .or_config_error()?;
        Ok(match input.family {
            Family::Embed => calls::embed(&auth, &input.step)?,
            Family::Rerank => runtime.block_on(calls::rerank(&auth, &input.step))?,
            Family::Audio => calls::audio(&auth, &input.step)?,
            Family::Passthrough => calls::passthrough(&auth, &input.step)?,
        })
    }
}
