#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};
use betterlite_policy::pipeline::PipelineMode;

use super::super::streaming::{
    PipelineCallbacks, StreamingSupport, pipeline_is_streamable, step_supports_streaming,
    stream_gated_guardrail_names, streamable_post_call_pipelines, unsupported_stream_guardrails,
};
use super::{capture_warnings, data_with, pipeline};

struct FakeTranslation<const ASSEMBLES: bool>;

impl<const ASSEMBLES: bool> BaseTranslation for FakeTranslation<ASSEMBLES> {
    type Guardrail = ();
    type LogTarget = ();
    type Failure = GuardrailFailure;

    const ASSEMBLES_STREAMED_RESPONSE: bool = ASSEMBLES;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }
}

struct FakeCallbacks(Vec<(&'static str, StreamingSupport)>);

impl PipelineCallbacks for FakeCallbacks {
    fn find_guardrail_callback(&self, guardrail_name: &str) -> Option<StreamingSupport> {
        self.0
            .iter()
            .find(|(name, _)| *name == guardrail_name)
            .map(|(_, support)| *support)
    }
}

fn callbacks(entries: &[(&'static str, bool, bool)]) -> FakeCallbacks {
    let supports = entries
        .iter()
        .map(|(name, unified, streaming)| {
            (
                *name,
                StreamingSupport {
                    unified_execution: *unified,
                    streaming_execution: *streaming,
                },
            )
        })
        .collect();
    FakeCallbacks(supports)
}

#[test]
fn step_supports_streaming_rejects_an_unknown_guardrail() {
    let callbacks = callbacks(&[]);
    let translation = FakeTranslation::<true>;
    assert!(!step_supports_streaming(
        "pii",
        Some(&translation),
        &callbacks
    ));
}

#[test]
fn step_supports_streaming_accepts_unified_execution() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let translation = FakeTranslation::<false>;
    assert!(step_supports_streaming::<FakeTranslation<false>>(
        "pii", None, &callbacks
    ));
    assert!(step_supports_streaming(
        "pii",
        Some(&translation),
        &callbacks
    ));
}

#[test]
fn step_supports_streaming_needs_a_translation_that_assembles_the_response() {
    let callbacks = callbacks(&[("pii", false, true)]);
    assert!(!step_supports_streaming::<FakeTranslation<false>>(
        "pii", None, &callbacks
    ));
    let plain = FakeTranslation::<false>;
    assert!(!step_supports_streaming("pii", Some(&plain), &callbacks));
    let assembling = FakeTranslation::<true>;
    assert!(step_supports_streaming(
        "pii",
        Some(&assembling),
        &callbacks
    ));
}

#[test]
fn step_supports_streaming_needs_streaming_execution() {
    let callbacks = callbacks(&[("pii", false, false)]);
    let assembling = FakeTranslation::<true>;
    assert!(!step_supports_streaming(
        "pii",
        Some(&assembling),
        &callbacks
    ));
}

#[test]
fn unsupported_stream_guardrails_keeps_first_seen_order_without_repeats() {
    let callbacks = callbacks(&[("pii", false, false), ("block", true, false)]);
    let assembling = FakeTranslation::<true>;
    let steps = pipeline(
        PipelineMode::PostCall,
        &["pii", "block", "pii", "moderation"],
    );
    assert_eq!(
        unsupported_stream_guardrails(&steps, Some(&assembling), &callbacks),
        ["pii", "moderation"]
    );
}

#[test]
fn unsupported_stream_guardrails_is_empty_when_every_step_runs() {
    let callbacks = callbacks(&[("pii", true, false), ("block", true, false)]);
    let steps = pipeline(PipelineMode::PostCall, &["pii", "block"]);
    assert!(
        unsupported_stream_guardrails::<FakeTranslation<false>>(&steps, None, &callbacks)
            .is_empty()
    );
}

#[test]
fn pipeline_is_streamable_warns_about_dropped_guardrails() {
    let callbacks = callbacks(&[]);
    let steps = pipeline(PipelineMode::PostCall, &["pii", "block"]);
    let (streamable, log) = capture_warnings(|| {
        pipeline_is_streamable::<FakeTranslation<false>>("policy-a", &steps, None, &callbacks)
    });
    assert!(!streamable);
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("policy-a"), "{log}");
    assert!(log.contains("pii, block"), "{log}");
}

#[test]
fn pipeline_is_streamable_is_quiet_when_every_step_runs() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let steps = pipeline(PipelineMode::PostCall, &["pii"]);
    let (streamable, log) = capture_warnings(|| {
        pipeline_is_streamable::<FakeTranslation<false>>("policy-a", &steps, None, &callbacks)
    });
    assert!(streamable);
    assert!(!log.contains("WARN"), "{log}");
}

#[test]
fn stream_gated_guardrail_names_needs_a_translation() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let request = data_with(&[("policy-a", pipeline(PipelineMode::PostCall, &["pii"]))]);
    let named = stream_gated_guardrail_names::<FakeTranslation<true>>(&request, None, &callbacks);
    assert!(named.is_empty());
}

#[test]
fn stream_gated_guardrail_names_keeps_only_streamable_post_call_pipelines() {
    let callbacks = callbacks(&[("pii", true, false), ("block", false, true)]);
    let assembling = FakeTranslation::<true>;
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        (
            "policy-b",
            pipeline(PipelineMode::PostCall, &["block", "moderation"]),
        ),
        ("policy-c", pipeline(PipelineMode::PreCall, &["moderation"])),
    ]);
    let named = stream_gated_guardrail_names(&request, Some(&assembling), &callbacks);
    assert_eq!(named, BTreeSet::from(["pii".to_owned()]));
}

#[test]
fn stream_gated_guardrail_names_requires_an_assembling_translation() {
    let callbacks = callbacks(&[("block", false, true)]);
    let plain = FakeTranslation::<false>;
    let request = data_with(&[("policy-b", pipeline(PipelineMode::PostCall, &["block"]))]);
    let named = stream_gated_guardrail_names(&request, Some(&plain), &callbacks);
    assert!(named.is_empty());
}

#[test]
fn streamable_post_call_pipelines_stays_quiet_without_post_call_pipelines() {
    let callbacks = callbacks(&[]);
    let request = data_with(&[("policy-a", pipeline(PipelineMode::PreCall, &["pii"]))]);
    let (pipelines, log) = capture_warnings(|| {
        streamable_post_call_pipelines::<FakeTranslation<true>>(
            &request,
            None,
            Some("/v1/responses"),
            &callbacks,
        )
    });
    assert!(pipelines.is_empty());
    assert!(!log.contains("WARN"), "{log}");
}

#[test]
fn streamable_post_call_pipelines_warns_with_the_route_without_a_translation() {
    let callbacks = callbacks(&[]);
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        ("policy-b", pipeline(PipelineMode::PostCall, &["block"])),
    ]);
    let (pipelines, log) = capture_warnings(|| {
        streamable_post_call_pipelines::<FakeTranslation<true>>(
            &request,
            None,
            Some("/v1/responses"),
            &callbacks,
        )
    });
    assert!(pipelines.is_empty());
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("/v1/responses"), "{log}");
    assert!(log.contains("policy-a, policy-b"), "{log}");
}

#[test]
fn streamable_post_call_pipelines_drops_an_unstreamable_pipeline() {
    let callbacks = callbacks(&[("pii", true, false)]);
    let assembling = FakeTranslation::<true>;
    let request = data_with(&[
        ("policy-a", pipeline(PipelineMode::PostCall, &["pii"])),
        (
            "policy-b",
            pipeline(PipelineMode::PostCall, &["pii", "block"]),
        ),
    ]);
    let (pipelines, log) = capture_warnings(|| {
        streamable_post_call_pipelines(
            &request,
            Some(&assembling),
            Some("/v1/responses"),
            &callbacks,
        )
    });
    let names: Vec<&str> = pipelines.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["policy-a"]);
    assert!(log.contains("WARN"), "{log}");
    assert!(log.contains("policy-b"), "{log}");
    assert!(log.contains("block"), "{log}");
}
