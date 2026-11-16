use std::future::{Future, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use secrecy::SecretString;
use serde_json::Value;

use crate::openai_base_config::Object;
use crate::watsonx_auth::WatsonxAuth;
use crate::watsonx_auth::environment::WatsonxEnvironment;
use crate::watsonx_auth::transport::WatsonxTransport;
use crate::watsonx_auth::types::WatsonxParams;

pub(super) const TOKEN_BODY: &str = r#"{"access_token":"iam-token","expires_in":3600}"#;

pub(super) struct CountingTransport {
    calls: AtomicUsize,
    reply: Option<&'static str>,
}

impl CountingTransport {
    pub(super) const fn answering(body: &'static str) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            reply: Some(body),
        }
    }

    pub(super) const fn closed() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            reply: None,
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl WatsonxTransport for CountingTransport {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(self.reply.map_or(Err(TransportError::Closed), |body| {
            Ok(UpstreamResponse {
                status: 200,
                headers: HeaderMap::new(),
                body: body.as_bytes().to_vec(),
            })
        }))
    }
}

pub(super) fn environment(api_base: Option<&str>, project_id: Option<&str>) -> WatsonxEnvironment {
    WatsonxEnvironment {
        watsonx_iam_url: None,
        wx_api_key: None,
        watsonx_api_key: None,
        watsonx_apikey: None,
        watsonx_zenapikey: None,
        watsonx_token: None,
        watsonx_api_base: api_base.map(str::to_owned),
        watsonx_url: None,
        wx_url: None,
        wml_url: None,
        watsonx_project_id: project_id.map(str::to_owned),
        wx_project_id: None,
        project_id: None,
        watsonx_region: None,
        wx_region: None,
        region: None,
        watsonx_deployment_space_id: None,
        watsonx_space_id: None,
        wx_space_id: None,
        space_id: None,
    }
}

pub(super) fn auth(
    environment: WatsonxEnvironment,
    transport: CountingTransport,
) -> WatsonxAuth<CountingTransport> {
    WatsonxAuth::new(environment, transport, Arc::new(|| 0.0_f64)).unwrap()
}

pub(super) fn object(value: Value) -> Object {
    let Value::Object(fields) = value else {
        panic!("the test value must be an object");
    };
    fields
}

pub(super) fn options(value: Value) -> WatsonxParams {
    object(value).into_iter().collect()
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}
