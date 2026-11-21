#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Context;
use opentelemetry::trace::{Span as _, TraceContextExt as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use rstest::rstest;

use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::fan_out::scope::RequestScopeSource as _;
use crate::otel::langfuse_root::RootSpanSource as _;
use crate::otel::logger::v2::RequestTraceSource as _;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::factory::build_otel_v2_logger;
use crate::otel::logger_runtime::tests::support::{NoSettings, NullAssembler, destination};
use crate::otel::logger_runtime::types::{CallbackHooks, ConstructorSettings, InjectedProviders};
use crate::otel::plumbing::context::context_slots::AnchoredSpan;
use crate::otel::proxy_runtime::error::ProxyRuntimeError;
use crate::otel::proxy_runtime::runtime::{ProxyRuntimeInputs, build_proxy_runtime};
use crate::otel::proxy_runtime::slot::{anchor_request_root, request_slot_scope, set_destinations};
use crate::otel::proxy_runtime::sources::ProxyTraceSources;

use super::support::{anchored, config, environment, http_clients, utc_offset};

const SPAN_LIMIT: &str = "OTEL_SPAN_ATTRIBUTE_COUNT_LIMIT";
const GLOBAL_LIMIT: &str = "OTEL_ATTRIBUTE_COUNT_LIMIT";

fn runtime_with(
    lookup: BTreeMap<String, String>,
    configured_exclusions: Option<Vec<String>>,
) -> Result<Arc<LoggerRuntime>, ProxyRuntimeError> {
    let loaded = config("");
    let environment: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(lookup);
    let sources = Arc::new(ProxyTraceSources::new(environment.as_ref()));
    build_proxy_runtime(ProxyRuntimeInputs {
        config: &loaded,
        environment,
        http_clients: http_clients(),
        callback_settings: Arc::new(NoSettings),
        configured_exclusions,
        assembler: Arc::new(NullAssembler),
        utc_offset: utc_offset(),
        sources,
    })
}

fn injected(provider: &SdkTracerProvider) -> InjectedProviders<'_> {
    InjectedProviders {
        tracer_provider: Some(provider),
        logger_provider: None,
        meter_provider: None,
    }
}

fn attribute_limit(runtime: &LoggerRuntime) -> Option<usize> {
    let provider = SdkTracerProvider::builder().build();
    let logger = build_otel_v2_logger(
        runtime,
        runtime.base.clone(),
        None,
        injected(&provider),
        ConstructorSettings::default(),
    )
    .expect("the runtime builds a logger");
    let Some(CallbackHooks::Generic(callback)) = logger.hooks.as_ref() else {
        panic!("a config without langfuse builds the generic V2Callback");
    };
    callback.logger.emitter.attribute_limit
}

#[rstest]
#[case::on(Some("true"), true)]
#[case::off(Some("false"), false)]
#[case::empty(Some(""), false)]
#[case::unset(None, false)]
fn the_v2_switch_follows_the_environment(#[case] value: Option<&str>, #[case] enabled: bool) {
    let lookup = match value {
        Some(value) => environment(&[("LITELLM_OTEL_V2", value)]),
        None => environment(&[]),
    };
    let runtime = runtime_with(lookup, None).expect("the runtime builds");
    assert_eq!(runtime.v2_enabled, enabled);
}

#[test]
fn a_non_flag_v2_switch_is_an_error() {
    let Err(error) = runtime_with(environment(&[("LITELLM_OTEL_V2", "sometimes")]), None) else {
        panic!("the switch refuses the value");
    };
    assert!(matches!(error, ProxyRuntimeError::V2Flag { .. }));
}

#[rstest]
#[case::configured(Some(vec!["postgresql".to_owned()]), Some(vec!["postgresql".to_owned()]))]
#[case::absent(None, None)]
fn the_configured_exclusions_pass_through(
    #[case] configured: Option<Vec<String>>,
    #[case] expected: Option<Vec<String>>,
) {
    let runtime = runtime_with(environment(&[]), configured).expect("the runtime builds");
    assert_eq!(runtime.configured_exclusions, expected);
}

#[rstest]
#[case::unset(&[], Some(128))]
#[case::span_variable(&[(SPAN_LIMIT, "64")], Some(64))]
#[case::global_variable(&[(GLOBAL_LIMIT, "32")], Some(32))]
#[case::span_wins(&[(SPAN_LIMIT, "64"), (GLOBAL_LIMIT, "32")], Some(64))]
#[case::empty_span(&[(SPAN_LIMIT, "")], None)]
#[case::empty_span_beats_the_global(&[(SPAN_LIMIT, " "), (GLOBAL_LIMIT, "32")], None)]
#[case::empty_global(&[(GLOBAL_LIMIT, "")], Some(128))]
#[case::padded_span(&[(SPAN_LIMIT, " 16 ")], Some(16))]
fn the_span_attribute_limit_follows_the_sdk_variables(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<usize>,
) {
    let runtime = runtime_with(environment(variables), None).expect("the runtime builds");
    assert_eq!(attribute_limit(&runtime), expected);
}

#[rstest]
#[case::negative_span(SPAN_LIMIT, "-1")]
#[case::negative_global(GLOBAL_LIMIT, "-2")]
#[case::not_a_number(GLOBAL_LIMIT, "many")]
#[case::not_a_number_span(SPAN_LIMIT, "many")]
fn a_bad_span_attribute_limit_is_an_error(#[case] variable: &str, #[case] value: &str) {
    let Err(error) = runtime_with(environment(&[(variable, value)]), None) else {
        panic!("the limit refuses the value");
    };
    assert!(matches!(
        error,
        ProxyRuntimeError::AttributeLimit { variable: refused } if refused == variable
    ));
}

#[tokio::test]
async fn the_runtime_reads_the_request_slot_through_its_sources() {
    let runtime = runtime_with(environment(&[]), None).expect("the runtime builds");
    let carried = vec![destination("langfuse_otel")];
    request_slot_scope(async {
        anchor_request_root(anchored(5, 5), Context::new());
        set_destinations(carried.clone());
        assert_eq!(runtime.trace.trace_context().root, Some(anchored(5, 5)));
        assert_eq!(runtime.scope.scope().destinations, carried);
    })
    .await;
}

#[tokio::test]
async fn a_langfuse_content_config_builds_the_content_logger_with_the_assembler() {
    let loaded = config(
        "callback_settings:\n  otel:\n    mapper_names: [langfuse]\n    capture_message_content: span_only\n",
    );
    let assembler: Arc<dyn StreamOutputAssembler> = Arc::new(NullAssembler);
    let sources = Arc::new(ProxyTraceSources::new(&BTreeMap::<String, String>::new()));
    let runtime = build_proxy_runtime(ProxyRuntimeInputs {
        config: &loaded,
        environment: Arc::new(BTreeMap::<String, String>::new()),
        http_clients: http_clients(),
        callback_settings: Arc::new(NoSettings),
        configured_exclusions: None,
        assembler: Arc::clone(&assembler),
        utc_offset: utc_offset(),
        sources,
    })
    .expect("the runtime builds");
    let provider = SdkTracerProvider::builder().build();
    let logger = build_otel_v2_logger(
        &runtime,
        runtime.base.clone(),
        Some("langfuse_otel"),
        injected(&provider),
        ConstructorSettings::default(),
    )
    .expect("the langfuse content logger builds");
    let Some(CallbackHooks::LangfuseContent(content)) = logger.hooks.as_ref() else {
        panic!("a langfuse_otel config with content capture builds LangfuseContentOtel");
    };
    assert!(Arc::ptr_eq(&assembler, &content.stream_assembler));
    request_slot_scope(async {
        let span = provider.tracer("proxy_runtime_test").start("SERVER");
        let root = AnchoredSpan {
            context: span.span_context().clone(),
            http_route: None,
        };
        anchor_request_root(root, Context::current_with_span(span));
        assert!(content.inner.roots.request_root_span().is_some());
    })
    .await;
}
