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
    let text = |name: &str| values.get(name).cloned();
    let secret = |name: &str| text(name).map(SecretString::from);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
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
