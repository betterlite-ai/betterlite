use std::collections::BTreeMap;
use std::future::Future;

use super::elicitation::{
    DownstreamElicitSession, ElicitRequestParams, ElicitResult, handle_elicitation_request,
};
use crate::wire_compat::capabilities::policy::ClientCapabilities;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallerCapture<Auth> {
    pub user_api_key_auth: Option<Auth>,
    pub raw_headers: Option<BTreeMap<String, String>>,
    pub client_ip: Option<String>,
}

impl<Auth: Clone> CallerCapture<Auth> {
    #[must_use]
    pub fn resolve(
        user_api_key_auth: Option<Auth>,
        raw_headers: Option<BTreeMap<String, String>>,
        client_ip: Option<String>,
        operation_context: Option<Self>,
        ambient: Option<&Self>,
    ) -> Self {
        if let Some(context) = operation_context {
            return context;
        }
        let fallback = user_api_key_auth.is_none().then_some(ambient).flatten();
        let (ambient_auth, ambient_headers, ambient_ip) = fallback.map_or_else(
            || (None, None, None),
            |context| {
                (
                    context.user_api_key_auth.clone(),
                    context.raw_headers.clone(),
                    context.client_ip.clone(),
                )
            },
        );
        Self {
            user_api_key_auth: user_api_key_auth.or(ambient_auth),
            raw_headers: raw_headers.or(ambient_headers),
            client_ip: client_ip.or(ambient_ip),
        }
    }
}

pub struct SamplingCall<'call, Auth, Context, Params> {
    pub context: &'call Context,
    pub params: &'call Params,
    pub default_model: Option<&'call str>,
    pub caller: &'call CallerCapture<Auth>,
}

pub trait SamplingHandler<Auth> {
    type Context;
    type Params;
    type Response;

    fn handle<'handler, 'call>(
        &'handler self,
        call: SamplingCall<'call, Auth, Self::Context, Self::Params>,
    ) -> impl Future<Output = Self::Response> + Send + 'handler + 'call;
}

pub trait SamplingCallback {
    type Context;
    type Params;
    type Response;

    fn call<'callback>(
        &'callback self,
        context: &'callback Self::Context,
        params: &'callback Self::Params,
    ) -> impl Future<Output = Self::Response> + Send + 'callback;
}

pub struct CapturedSampling<Auth, Handler> {
    pub handler: Handler,
    pub caller: CallerCapture<Auth>,
    pub default_model: Option<String>,
}

impl<Auth, Handler> SamplingCallback for CapturedSampling<Auth, Handler>
where
    Auth: Sync,
    Handler: SamplingHandler<Auth> + Sync,
    Handler::Context: Sync,
    Handler::Params: Sync,
{
    type Context = Handler::Context;
    type Params = Handler::Params;
    type Response = Handler::Response;

    fn call<'callback>(
        &'callback self,
        context: &'callback Self::Context,
        params: &'callback Self::Params,
    ) -> impl Future<Output = Self::Response> + Send + 'callback {
        let call = SamplingCall {
            context,
            params,
            default_model: self.default_model.as_deref(),
            caller: &self.caller,
        };
        async move { self.handler.handle(call).await }
    }
}

pub trait ElicitationCallback {
    fn answer<'callback>(
        &'callback self,
        params: &'callback ElicitRequestParams,
    ) -> impl Future<Output = ElicitResult> + Send + 'callback;
}

pub struct CapturedElicitation<Session> {
    pub session: Option<Session>,
    pub capabilities: Option<ClientCapabilities>,
}

impl<Session> ElicitationCallback for CapturedElicitation<Session>
where
    Session: DownstreamElicitSession + Sync,
{
    fn answer<'callback>(
        &'callback self,
        params: &'callback ElicitRequestParams,
    ) -> impl Future<Output = ElicitResult> + Send + 'callback {
        handle_elicitation_request(params, self.session.as_ref(), self.capabilities.as_ref())
    }
}

mod tests;
