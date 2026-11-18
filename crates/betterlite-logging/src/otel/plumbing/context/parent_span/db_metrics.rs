use betterlite_db::database_failures::metrics::ProxySpanResolver;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use opentelemetry::trace::SpanContext;
use serde::Deserialize;

use crate::otel::model::metadata::wire::callback::Params;
use crate::otel::model::metadata::wire::decode;
use crate::otel::model::metadata::wire::payload::Metadata;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::context::parent_span::{
    ParentSpanSources, parent_otel_span_from_kwargs,
};

// the kwargs log_db_metrics passes for a PROXY call, read through B3847's metadata precedence
#[derive(Deserialize)]
struct MetricKwargs {
    metadata: Option<Metadata>,
    litellm_params: Option<Params>,
    litellm_parent_otel_span: Option<OtelModelValue>,
}

pub struct KwargsParentSpanResolver;

impl ProxySpanResolver for KwargsParentSpanResolver {
    fn parent_span(&self, passed_kwargs: &IndexMap<String, Json>) -> Option<SpanContext> {
        let text = serde_json::to_string(passed_kwargs).ok()?;
        let kwargs: MetricKwargs = decode(&text).ok()?;
        parent_otel_span_from_kwargs(&ParentSpanSources {
            metadata: kwargs.metadata.as_ref(),
            params: kwargs.litellm_params.as_ref(),
            span: kwargs.litellm_parent_otel_span.as_ref(),
        })
    }
}

#[path = "db_metrics/tests.rs"]
mod tests;
