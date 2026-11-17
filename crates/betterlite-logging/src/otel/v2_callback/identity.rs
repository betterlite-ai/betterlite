use std::collections::BTreeMap;

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

// the auth boundary's identity inputs; with no root anchored yet the ambient server span takes the stamps
#[derive(Clone, Copy)]
pub struct IdentitySeed<'a> {
    pub auth: &'a Auth,
    pub model: Option<&'a str>,
    pub request_metadata: Option<&'a MetadataMap>,
    pub ambient_span: Option<&'a LiveSpan>,
}

impl V2Callback {
    // litellm.integrations.otel.logger.OpenTelemetryV2.seed_request_identity
    #[must_use]
    pub fn seed_request_identity(
        &self,
        seed: &IdentitySeed<'_>,
        trace: &mut RequestTraceContext,
        context: &Context,
    ) -> Context {
        let identity = from_user_api_key_auth(seed.auth, seed.request_metadata);
        let bag = promoted_baggage(
            &identity,
            seed.model,
            &self.logger.config.baggage_promoted_keys,
            &self.logger.config.baggage_metadata_keys,
            &self.logger.config.baggage_team_metadata_keys,
        );
        if bag.is_empty() {
            return context.clone();
        }
        self.stamp_identity(&bag, trace, seed.ambient_span);
        set_request_baggage(&bag, Some(context))
    }

    // litellm.integrations.otel.logger.OpenTelemetryV2.async_pre_call_hook, sync here because it does no I/O
    #[must_use]
    pub fn async_pre_call_hook(
        &self,
        auth: &Auth,
        data: &RequestData,
        ambient_span: Option<&LiveSpan>,
        trace: &mut RequestTraceContext,
        context: &Context,
    ) -> Context {
        let metadata = metadata_from_request_data(data);
        let seed = IdentitySeed {
            auth,
            model: data.model.as_deref(),
            request_metadata: metadata.as_ref(),
            ambient_span,
        };
        self.seed_request_identity(&seed, trace, context)
    }

    // the Baggage processor only fires at span start, so the server span already open is stamped here
    fn stamp_identity(
        &self,
        bag: &BTreeMap<String, String>,
        trace: &mut RequestTraceContext,
        ambient: Option<&LiveSpan>,
    ) {
        let root = self.request_root_span(trace);
        let Some(span) = root.as_ref().or(ambient) else {
            return;
        };
        if !span.context.is_valid() {
            return;
        }
        self.mark_request_root_span(span, None, trace);
        for (key, value) in bag {
            span.set_attribute(KeyValue::new(key.clone(), value.clone()));
        }
    }
}
