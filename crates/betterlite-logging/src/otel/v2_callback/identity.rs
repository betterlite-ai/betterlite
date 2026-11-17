use std::collections::BTreeMap;

use opentelemetry::trace::Span as _;
use opentelemetry::{Context, KeyValue};

use crate::otel::model::baggage::promoted_baggage;
use crate::otel::model::metadata::extraction::metadata_from_request_data;
use crate::otel::model::metadata::request_identity::from_auth::from_user_api_key_auth;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::metadata::wire::callback::RequestData;
use crate::otel::model::metadata::wire::payload::Auth;
use crate::otel::plumbing::context::baggage_context::set_request_baggage;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::callback::V2Callback;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::proxy;

// the auth boundary's identity inputs; with no root anchored yet the ambient server span takes the stamps
#[derive(Clone, Copy)]
pub struct IdentitySeed<'seed> {
    pub auth: &'seed Auth,
    pub model: Option<&'seed str>,
    pub request_metadata: Option<&'seed MetadataMap>,
    pub ambient_span: Option<&'seed LiveSpan>,
}

// litellm.integrations.otel.logger.OpenTelemetryV2.seed_request_identity
#[must_use]
pub fn seed_request_identity(
    callback: &V2Callback,
    seed: &IdentitySeed<'_>,
    trace: &mut RequestTraceContext,
) -> Context {
    let identity = from_user_api_key_auth(seed.auth, seed.request_metadata);
    let bag = promoted_baggage(
        &identity,
        seed.model,
        &callback.logger.config.baggage_promoted_keys,
        &callback.logger.config.baggage_metadata_keys,
        &callback.logger.config.baggage_team_metadata_keys,
    );
    if bag.is_empty() {
        return Context::current();
    }
    stamp_identity(callback, &bag, trace, seed.ambient_span);
    set_request_baggage(&bag, None)
}

// litellm.integrations.otel.logger.OpenTelemetryV2.async_pre_call_hook, sync here because it does no I/O
#[must_use]
pub fn async_pre_call_hook(
    callback: &V2Callback,
    auth: &Auth,
    data: &RequestData,
    ambient_span: Option<&LiveSpan>,
    trace: &mut RequestTraceContext,
) -> Context {
    let metadata = metadata_from_request_data(data);
    let seed = IdentitySeed {
        auth,
        model: data.model.as_deref(),
        request_metadata: metadata.as_ref(),
        ambient_span,
    };
    seed_request_identity(callback, &seed, trace)
}

// the Baggage processor only fires at span start, so the server span already open is stamped here
fn stamp_identity(
    callback: &V2Callback,
    bag: &BTreeMap<String, String>,
    trace: &mut RequestTraceContext,
    ambient: Option<&LiveSpan>,
) {
    let root = proxy::request_root_span(callback, trace);
    let Some(span) = root.as_ref().or(ambient) else {
        return;
    };
    if !span.context.is_valid() {
        return;
    }
    proxy::mark_request_root_span(callback, span, None, trace);
    for (key, value) in bag {
        span.lock()
            .set_attribute(KeyValue::new(key.clone(), value.clone()));
    }
}
