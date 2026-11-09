#![cfg(test)]

use std::collections::BTreeMap;
use std::iter;
use std::slice;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::DynamicCallback;
use super::DynamicCallbackKind;
use super::DynamicCallbackLists;
use super::MAX_CALLBACKS_PER_LIST;
use super::error::DynamicCallbackError;
use super::filters::get_callback_name;
use super::filters::get_combined_callback_list;
use super::filters::is_internal_callback;
use super::filters::remove_internal_callbacks;
use super::filters::remove_logger_callbacks;
use super::params::standard_callback_params;
use super::resolve::resolve_callback_string;
use super::with_dynamic_callback;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::logging_privacy::CallbackMetadataError;
use crate::logging_privacy::PrivacyContext;
use crate::logging_privacy::callbacks::TrustedCallbacks;
use crate::logging_privacy::request::PrivacyRequest;
use crate::logging_state::types::DynamicCallbacks;
use crate::registry::error::RegistryError;

struct MarkerLogger {
    label: String,
}

#[async_trait]
impl CallbackLogger for MarkerLogger {
    fn callback_name(&self) -> &str {
        &self.label
    }
}

enum Outcome {
    Logger(Arc<dyn CallbackLogger>),
    Split {
        with_params: Arc<dyn CallbackLogger>,
        without_params: Arc<dyn CallbackLogger>,
    },
    Missing,
    Error,
}

struct Recorded {
    name: String,
    params: BTreeMap<String, String>,
}

struct FakeResolver {
    outcomes: BTreeMap<String, Outcome>,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeResolver {
    fn new(outcomes: Vec<(&str, Outcome)>) -> Self {
        Self {
            outcomes: outcomes
                .into_iter()
                .map(|(name, outcome)| (name.to_owned(), outcome))
                .collect(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Recorded> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls
            .iter()
            .map(|call| Recorded {
                name: call.name.clone(),
                params: call.params.clone(),
            })
            .collect()
    }
}

impl LoggerResolver for FakeResolver {
    fn resolve(
        &self,
        name: &str,
        params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                name: name.to_owned(),
                params: params.clone(),
            });
        match self.outcomes.get(name) {
            Some(Outcome::Logger(logger)) => Ok(Some(Arc::clone(logger))),
            Some(Outcome::Split {
                with_params,
                without_params,
            }) => Ok(Some(Arc::clone(if params.is_empty() {
                without_params
            } else {
                with_params
            }))),
            Some(Outcome::Missing) => Ok(None),
            Some(Outcome::Error) | None => Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            }),
        }
    }
}

fn marker(label: &str) -> Arc<dyn CallbackLogger> {
    Arc::new(MarkerLogger {
        label: label.to_owned(),
    })
}

fn name(value: &str) -> DynamicCallback {
    DynamicCallback::Name(value.to_owned())
}

fn logger(label: &str) -> DynamicCallback {
    DynamicCallback::Logger(marker(label))
}

fn labels(callbacks: &[DynamicCallback]) -> Vec<String> {
    callbacks
        .iter()
        .map(|callback| get_callback_name(callback).to_owned())
        .collect()
}

fn resolved_labels(loggers: &[Arc<dyn CallbackLogger>]) -> Vec<String> {
    loggers
        .iter()
        .map(|logger| logger.callback_name().to_owned())
        .collect()
}

fn lists() -> DynamicCallbackLists {
    DynamicCallbackLists {
        input: None,
        success: None,
        async_success: None,
        failure: None,
        async_failure: None,
    }
}

fn trusted(pairs: &[(&str, &str)]) -> TrustedCallbacks {
    TrustedCallbacks {
        values: pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
            .collect(),
    }
}

fn request(value: &str) -> PrivacyRequest {
    serde_json::from_str(value).expect("privacy request decodes")
}

fn request_params(
    value: &str,
    trusted: &TrustedCallbacks,
    disabled: bool,
) -> Result<IndexMap<String, Json>, DynamicCallbackError> {
    standard_callback_params(
        &request(value),
        trusted,
        PrivacyContext {
            message_logging_disabled: disabled,
        },
    )
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn all_known(_name: &str) -> bool {
    true
}

fn marker_list(count: usize) -> Vec<DynamicCallback> {
    (0..count)
        .map(|index| logger(&format!("l{index}")))
        .collect()
}

fn name_list(count: usize) -> Vec<DynamicCallback> {
    iter::repeat_with(|| name("langfuse")).take(count).collect()
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    AddRegistersInEveryList,
    AddKeepsOneCopy,
    WithDynamicCallbackAppendsOnce,
    ProcessInputResolvesNamesOnly,
    ProcessDropsAnUnavailableName,
    ProcessKeepsNoneAndEmptyLists,
    ProcessAcceptsAListAtTheLimit,
    ProcessRejectsAListOverTheLimit,
    ProcessRejectsAnOverLimitListBeforeResolving,
    FailureAtTheLimitAcceptsAnAsyncMirror,
    SuccessMirrorsIntoAsyncSuccess,
    SuccessDuplicatesAnAlreadyListedAsyncEntry,
    SuccessKeepsANonKnownNameWithoutMirroring,
    FailureMirrorsIntoAsyncFailure,
    ProcessErrorNamesTheCallback,
    ResolveDatadogSendsOnlyDdVars,
    ResolveNewrelicKeepsTeamAndTraceLoggers,
    ResolveNewrelicKeepsOneLoggerWhenTheResolverReturnsTheSameArc,
    ResolveNewrelicWithoutAKeyResolvesOnce,
    ResolveNoneAddsNothing,
    ResolveErrorNamesTheCallback,
    CombinedWithoutDynamicCopiesGlobal,
    CombinedDeduplicatesDynamicFirst,
    CombinedWithEmptyDynamicDeduplicatesGlobal,
    CallbackNameReadsNamesAndLoggers,
    DebugShowsNamesAndLoggerNames,
    InternalCallbackFilterKeepsNamesAndDropsLoggers,
    UnknownCallbackNameSurvivesTheLoggerFilter,
    ParamsPrecedenceFollowsRequestSlots,
    ParamsPrecedenceSkipsAMissingSlot,
    ParamsSkipUnsupportedAndBlockedFields,
    ParamsApplyTrustedOverlay,
    ParamsInheritedPrivacyWins,
    ParamsRejectEnvironmentReferences,
    ParamsRejectAShadowedEnvironmentReference,
    ParamsRejectAnInvalidLangfuseEnvironment,
    ParamsEmptyRequestIsEmpty,
    WireNamesConvertToListEntries,
}

#[rstest]
#[case::add_registers_in_every_list(Scenario::AddRegistersInEveryList)]
#[case::add_keeps_one_copy(Scenario::AddKeepsOneCopy)]
#[case::with_dynamic_callback_appends_once(Scenario::WithDynamicCallbackAppendsOnce)]
#[case::process_input_resolves_names_only(Scenario::ProcessInputResolvesNamesOnly)]
#[case::process_drops_an_unavailable_name(Scenario::ProcessDropsAnUnavailableName)]
#[case::process_keeps_none_and_empty_lists(Scenario::ProcessKeepsNoneAndEmptyLists)]
#[case::process_accepts_a_list_at_the_limit(Scenario::ProcessAcceptsAListAtTheLimit)]
#[case::process_rejects_a_list_over_the_limit(Scenario::ProcessRejectsAListOverTheLimit)]
#[case::process_rejects_an_over_limit_list_before_resolving(
    Scenario::ProcessRejectsAnOverLimitListBeforeResolving
)]
#[case::failure_at_the_limit_accepts_an_async_mirror(
    Scenario::FailureAtTheLimitAcceptsAnAsyncMirror
)]
#[case::success_mirrors_into_async_success(Scenario::SuccessMirrorsIntoAsyncSuccess)]
#[case::success_duplicates_an_already_listed_async_entry(
    Scenario::SuccessDuplicatesAnAlreadyListedAsyncEntry
)]
#[case::success_keeps_a_non_known_name_without_mirroring(
    Scenario::SuccessKeepsANonKnownNameWithoutMirroring
)]
#[case::failure_mirrors_into_async_failure(Scenario::FailureMirrorsIntoAsyncFailure)]
#[case::process_error_names_the_callback(Scenario::ProcessErrorNamesTheCallback)]
#[case::resolve_datadog_sends_only_dd_vars(Scenario::ResolveDatadogSendsOnlyDdVars)]
#[case::resolve_newrelic_keeps_team_and_trace_loggers(
    Scenario::ResolveNewrelicKeepsTeamAndTraceLoggers
)]
#[case::resolve_newrelic_keeps_one_logger_when_the_resolver_returns_the_same_arc(
    Scenario::ResolveNewrelicKeepsOneLoggerWhenTheResolverReturnsTheSameArc
)]
#[case::resolve_newrelic_without_a_key_resolves_once(
    Scenario::ResolveNewrelicWithoutAKeyResolvesOnce
)]
#[case::resolve_none_adds_nothing(Scenario::ResolveNoneAddsNothing)]
#[case::resolve_error_names_the_callback(Scenario::ResolveErrorNamesTheCallback)]
#[case::combined_without_dynamic_copies_global(Scenario::CombinedWithoutDynamicCopiesGlobal)]
#[case::combined_deduplicates_dynamic_first(Scenario::CombinedDeduplicatesDynamicFirst)]
#[case::combined_with_empty_dynamic_deduplicates_global(
    Scenario::CombinedWithEmptyDynamicDeduplicatesGlobal
)]
#[case::callback_name_reads_names_and_loggers(Scenario::CallbackNameReadsNamesAndLoggers)]
#[case::debug_shows_names_and_logger_names(Scenario::DebugShowsNamesAndLoggerNames)]
#[case::internal_callback_filter_keeps_names_and_drops_loggers(
    Scenario::InternalCallbackFilterKeepsNamesAndDropsLoggers
)]
#[case::unknown_callback_name_survives_the_logger_filter(
    Scenario::UnknownCallbackNameSurvivesTheLoggerFilter
)]
#[case::params_precedence_follows_request_slots(Scenario::ParamsPrecedenceFollowsRequestSlots)]
#[case::params_precedence_skips_a_missing_slot(Scenario::ParamsPrecedenceSkipsAMissingSlot)]
#[case::params_skip_unsupported_and_blocked_fields(Scenario::ParamsSkipUnsupportedAndBlockedFields)]
#[case::params_apply_trusted_overlay(Scenario::ParamsApplyTrustedOverlay)]
#[case::params_inherited_privacy_wins(Scenario::ParamsInheritedPrivacyWins)]
#[case::params_reject_environment_references(Scenario::ParamsRejectEnvironmentReferences)]
#[case::params_reject_a_shadowed_environment_reference(
    Scenario::ParamsRejectAShadowedEnvironmentReference
)]
#[case::params_reject_an_invalid_langfuse_environment(
    Scenario::ParamsRejectAnInvalidLangfuseEnvironment
)]
#[case::params_empty_request_is_empty(Scenario::ParamsEmptyRequestIsEmpty)]
#[case::wire_names_convert_to_list_entries(Scenario::WireNamesConvertToListEntries)]
fn dynamic_callbacks_behavior_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::AddRegistersInEveryList => {
            let mut lists = lists();
            lists.input = Some(vec![name("langfuse")]);
            lists.async_success = Some(vec![logger("otel")]);
            lists.failure = Some(Vec::new());
            let callback = logger("s3_v2");
            lists.add_dynamic_callback(&callback);
            lists.add_dynamic_callback(&callback);
            assert_eq!(
                labels(lists.input.as_deref().unwrap()),
                ["langfuse", "s3_v2"]
            );
            assert_eq!(labels(lists.success.as_deref().unwrap()), ["s3_v2"]);
            assert_eq!(
                labels(lists.async_success.as_deref().unwrap()),
                ["otel", "s3_v2"]
            );
            assert_eq!(labels(lists.failure.as_deref().unwrap()), ["s3_v2"]);
            assert_eq!(labels(lists.async_failure.as_deref().unwrap()), ["s3_v2"]);
        }
        Scenario::AddKeepsOneCopy => {
            let callback = logger("langfuse");
            let existing = [callback.clone()];
            assert_eq!(
                with_dynamic_callback(Some(&existing), &callback),
                slice::from_ref(&callback)
            );
            assert_eq!(with_dynamic_callback(None, &callback), [callback]);
        }
        Scenario::WithDynamicCallbackAppendsOnce => {
            assert_eq!(
                with_dynamic_callback(Some(&[name("langfuse")]), &name("langfuse")),
                [name("langfuse")]
            );
            assert_eq!(
                with_dynamic_callback(Some(&[name("langfuse")]), &name("datadog")),
                [name("langfuse"), name("datadog")]
            );
        }
        Scenario::ProcessInputResolvesNamesOnly => {
            let mut lists = lists();
            let callback = logger("otel");
            lists.input = Some(vec![name("langfuse"), callback]);
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            lists
                .process_dynamic_callback_list(
                    DynamicCallbackKind::Input,
                    &resolver,
                    &TrustedCallbacks::default(),
                    &all_known,
                )
                .unwrap();
            assert_eq!(
                labels(lists.input.as_deref().unwrap()),
                ["langfuse", "otel"]
            );
            assert_eq!(resolver.calls().len(), 1);
            assert!(resolver.calls().first().unwrap().params.is_empty());
        }
        Scenario::ProcessDropsAnUnavailableName => {
            let mut lists = lists();
            lists.input = Some(vec![name("gitlab"), name("langfuse")]);
            let resolver = FakeResolver::new(vec![
                ("gitlab", Outcome::Missing),
                ("langfuse", Outcome::Logger(marker("langfuse"))),
            ]);
            let known = |candidate: &str| matches!(candidate, "gitlab" | "langfuse");
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &known)
                .unwrap();
            assert_eq!(labels(lists.input.as_deref().unwrap()), ["langfuse"]);
            assert_eq!(resolver.calls().len(), 2);
        }
        Scenario::ProcessKeepsNoneAndEmptyLists => {
            let mut lists = lists();
            lists.success = Some(Vec::new());
            let resolver = FakeResolver::new(Vec::new());
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap();
            assert!(lists.input.is_none());
            assert_eq!(lists.success, Some(Vec::new()));
            assert!(lists.async_success.is_none());
            assert!(resolver.calls().is_empty());
        }
        Scenario::ProcessAcceptsAListAtTheLimit => {
            let mut lists = lists();
            lists.input = Some(marker_list(MAX_CALLBACKS_PER_LIST));
            let resolver = FakeResolver::new(Vec::new());
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap();
            assert_eq!(
                lists.input.as_deref().unwrap().len(),
                MAX_CALLBACKS_PER_LIST
            );
            assert!(resolver.calls().is_empty());
        }
        Scenario::ProcessRejectsAListOverTheLimit => {
            let mut lists = lists();
            lists.input = Some(name_list(MAX_CALLBACKS_PER_LIST.saturating_add(1)));
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            let error = lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap_err();
            assert!(matches!(
                &error,
                DynamicCallbackError::TooMany { list, .. } if *list == "input"
            ));
            assert!(error.to_string().contains("input"));
            assert!(resolver.calls().is_empty());
        }
        Scenario::ProcessRejectsAnOverLimitListBeforeResolving => {
            let mut lists = lists();
            lists.input = Some(vec![name("langfuse")]);
            lists.async_success = Some(marker_list(MAX_CALLBACKS_PER_LIST.saturating_add(1)));
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            let error = lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap_err();
            assert!(matches!(
                &error,
                DynamicCallbackError::TooMany { list, .. } if *list == "async_success"
            ));
            assert!(resolver.calls().is_empty());
        }
        Scenario::FailureAtTheLimitAcceptsAnAsyncMirror => {
            let mut lists = lists();
            let mut failure = vec![name("langfuse")];
            failure.extend(marker_list(MAX_CALLBACKS_PER_LIST.saturating_sub(1)));
            lists.failure = Some(failure);
            lists.async_failure = Some(marker_list(MAX_CALLBACKS_PER_LIST));
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap();
            assert_eq!(
                lists.failure.as_deref().unwrap().len(),
                MAX_CALLBACKS_PER_LIST
            );
            assert_eq!(
                lists.async_failure.as_deref().unwrap().len(),
                MAX_CALLBACKS_PER_LIST.saturating_add(1)
            );
            assert_eq!(resolver.calls().len(), 1);
        }
        Scenario::SuccessMirrorsIntoAsyncSuccess => {
            let mut lists = lists();
            lists.success = Some(vec![name("langfuse")]);
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap();
            assert_eq!(labels(lists.success.as_deref().unwrap()), ["langfuse"]);
            assert_eq!(
                labels(lists.async_success.as_deref().unwrap()),
                ["langfuse"]
            );
            assert_eq!(resolver.calls().len(), 1);
        }
        Scenario::SuccessDuplicatesAnAlreadyListedAsyncEntry => {
            let mut lists = lists();
            lists.success = Some(vec![name("langfuse")]);
            lists.async_success = Some(vec![name("langfuse")]);
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap();
            assert_eq!(labels(lists.success.as_deref().unwrap()), ["langfuse"]);
            assert_eq!(
                labels(lists.async_success.as_deref().unwrap()),
                ["langfuse", "langfuse"]
            );
            assert_eq!(resolver.calls().len(), 2);
        }
        Scenario::SuccessKeepsANonKnownNameWithoutMirroring => {
            let mut lists = lists();
            lists.success = Some(vec![name("helicone"), name("langfuse"), logger("otel")]);
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            let known = |candidate: &str| candidate == "langfuse";
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &known)
                .unwrap();
            assert_eq!(
                labels(lists.success.as_deref().unwrap()),
                ["helicone", "langfuse", "otel"]
            );
            assert_eq!(
                labels(lists.async_success.as_deref().unwrap()),
                ["langfuse"]
            );
            assert_eq!(resolver.calls().len(), 1);
        }
        Scenario::FailureMirrorsIntoAsyncFailure => {
            let mut lists = lists();
            lists.failure = Some(vec![name("langfuse")]);
            let resolver =
                FakeResolver::new(vec![("langfuse", Outcome::Logger(marker("langfuse")))]);
            lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap();
            assert_eq!(labels(lists.failure.as_deref().unwrap()), ["langfuse"]);
            assert_eq!(
                labels(lists.async_failure.as_deref().unwrap()),
                ["langfuse"]
            );
            assert!(lists.async_success.is_none());
        }
        Scenario::ProcessErrorNamesTheCallback => {
            let mut lists = lists();
            lists.input = Some(vec![name("langfuse")]);
            let resolver = FakeResolver::new(vec![("langfuse", Outcome::Error)]);
            let error = lists
                .process_dynamic_callbacks(&resolver, &TrustedCallbacks::default(), &all_known)
                .unwrap_err();
            assert!(matches!(
                &error,
                DynamicCallbackError::Unavailable { callback, .. } if callback == "langfuse"
            ));
            assert!(error.to_string().contains("langfuse"));
        }
        Scenario::ResolveDatadogSendsOnlyDdVars => {
            let resolver = FakeResolver::new(vec![("datadog", Outcome::Logger(marker("datadog")))]);
            let team = trusted(&[
                ("dd_api_key", "key"),
                ("dd_site", "site"),
                ("newrelic_api_key", "other"),
                ("langfuse_public_key", "pk"),
            ]);
            let loggers = resolve_callback_string("datadog", &team, &resolver).unwrap();
            assert_eq!(resolved_labels(&loggers), ["datadog"]);
            let calls = resolver.calls();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls.first().unwrap().name, "datadog");
            assert_eq!(
                calls.first().unwrap().params,
                BTreeMap::from([
                    ("dd_api_key".to_owned(), "key".to_owned()),
                    ("dd_site".to_owned(), "site".to_owned()),
                ])
            );
        }
        Scenario::ResolveNewrelicKeepsTeamAndTraceLoggers => {
            let resolver = FakeResolver::new(vec![(
                "newrelic",
                Outcome::Split {
                    with_params: marker("newrelic-metrics"),
                    without_params: marker("newrelic-trace"),
                },
            )]);
            let team = trusted(&[
                ("newrelic_api_key", "key"),
                ("newrelic_region", "eu"),
                ("dd_api_key", "other"),
            ]);
            let loggers = resolve_callback_string("newrelic", &team, &resolver).unwrap();
            assert_eq!(
                resolved_labels(&loggers),
                ["newrelic-metrics", "newrelic-trace"]
            );
            let calls = resolver.calls();
            assert_eq!(calls.len(), 2);
            assert_eq!(
                calls.first().unwrap().params,
                BTreeMap::from([
                    ("newrelic_api_key".to_owned(), "key".to_owned()),
                    ("newrelic_region".to_owned(), "eu".to_owned()),
                ])
            );
            assert!(calls[1].params.is_empty());
        }
        Scenario::ResolveNewrelicKeepsOneLoggerWhenTheResolverReturnsTheSameArc => {
            let same = marker("newrelic");
            let expected = Arc::clone(&same);
            let resolver = FakeResolver::new(vec![(
                "newrelic",
                Outcome::Split {
                    with_params: Arc::clone(&same),
                    without_params: same,
                },
            )]);
            let team = trusted(&[("newrelic_api_key", "key")]);
            let loggers = resolve_callback_string("newrelic", &team, &resolver).unwrap();
            assert_eq!(loggers.len(), 1);
            assert!(Arc::ptr_eq(loggers.first().unwrap(), &expected));
        }
        Scenario::ResolveNewrelicWithoutAKeyResolvesOnce => {
            let resolver = FakeResolver::new(vec![(
                "newrelic",
                Outcome::Split {
                    with_params: marker("newrelic-metrics"),
                    without_params: marker("newrelic-trace"),
                },
            )]);
            let team = trusted(&[("newrelic_region", "eu")]);
            let loggers = resolve_callback_string("newrelic", &team, &resolver).unwrap();
            assert_eq!(resolved_labels(&loggers), ["newrelic-metrics"]);
            assert_eq!(resolver.calls().len(), 1);
        }
        Scenario::ResolveNoneAddsNothing => {
            let resolver = FakeResolver::new(vec![("gitlab", Outcome::Missing)]);
            let loggers =
                resolve_callback_string("gitlab", &TrustedCallbacks::default(), &resolver).unwrap();
            assert!(loggers.is_empty());
        }
        Scenario::ResolveErrorNamesTheCallback => {
            let resolver = FakeResolver::new(vec![("langfuse", Outcome::Error)]);
            let error =
                resolve_callback_string("langfuse", &TrustedCallbacks::default(), &resolver)
                    .err()
                    .expect("the resolver error is reported");
            assert!(matches!(
                &error,
                DynamicCallbackError::Unavailable { callback, .. } if callback == "langfuse"
            ));
            assert!(error.to_string().contains("langfuse"));
        }
        Scenario::CombinedWithoutDynamicCopiesGlobal => {
            let global = [name("a"), name("a"), name("b")];
            assert_eq!(get_combined_callback_list(None, &global), global);
        }
        Scenario::CombinedDeduplicatesDynamicFirst => {
            let dynamic = [name("a"), name("b"), name("a")];
            let global = [name("b"), name("c")];
            assert_eq!(
                labels(&get_combined_callback_list(Some(&dynamic), &global)),
                ["a", "b", "c"]
            );
        }
        Scenario::CombinedWithEmptyDynamicDeduplicatesGlobal => {
            let global = [name("a"), name("a")];
            assert_eq!(
                labels(&get_combined_callback_list(Some(&[]), &global)),
                ["a"]
            );
        }
        Scenario::CallbackNameReadsNamesAndLoggers => {
            assert_eq!(get_callback_name(&name("langfuse")), "langfuse");
            assert_eq!(get_callback_name(&logger("s3_v2")), "s3_v2");
        }
        Scenario::DebugShowsNamesAndLoggerNames => {
            assert_eq!(format!("{:?}", name("langfuse")), "Name(\"langfuse\")");
            assert_eq!(format!("{:?}", logger("otel")), "Logger(\"otel\")");
        }
        Scenario::InternalCallbackFilterKeepsNamesAndDropsLoggers => {
            let entries = [name("langfuse"), logger("s3_v2")];
            assert!(!is_internal_callback(&entries[0]));
            assert!(is_internal_callback(&entries[1]));
            assert_eq!(labels(&remove_internal_callbacks(&entries)), ["langfuse"]);
            let known = |candidate: &str| candidate == "langfuse";
            assert!(remove_logger_callbacks(&entries, &known).is_empty());
        }
        Scenario::UnknownCallbackNameSurvivesTheLoggerFilter => {
            let entries = [name("langfuse"), name("not_a_callback"), logger("s3_v2")];
            let known = |candidate: &str| candidate == "langfuse";
            assert_eq!(
                labels(&remove_logger_callbacks(&entries, &known)),
                ["not_a_callback"]
            );
            assert_eq!(
                labels(&remove_internal_callbacks(&entries)),
                ["langfuse", "not_a_callback"]
            );
        }
        Scenario::ParamsPrecedenceFollowsRequestSlots => {
            let standard = request_params(
                r#"{
                    "metadata": {"langsmith_project": "metadata", "langfuse_public_key": "pk"},
                    "litellm_metadata": {"langsmith_project": "litellm_metadata", "langsmith_api_key": "key"},
                    "litellm_params": {"metadata": {"langsmith_project": "params", "arize_api_key": "arize"}}
                }"#,
                &TrustedCallbacks::default(),
                false,
            )
            .unwrap();
            assert_eq!(standard.get("langsmith_project"), Some(&text("params")));
            assert_eq!(standard.get("langfuse_public_key"), Some(&text("pk")));
            assert_eq!(standard.get("langsmith_api_key"), Some(&text("key")));
            assert_eq!(standard.get("arize_api_key"), Some(&text("arize")));
        }
        Scenario::ParamsPrecedenceSkipsAMissingSlot => {
            let standard = request_params(
                r#"{
                    "metadata": {"langsmith_project": "metadata"},
                    "litellm_metadata": {"langsmith_project": "litellm_metadata"}
                }"#,
                &TrustedCallbacks::default(),
                false,
            )
            .unwrap();
            assert_eq!(
                standard.get("langsmith_project"),
                Some(&text("litellm_metadata"))
            );
        }
        Scenario::ParamsSkipUnsupportedAndBlockedFields => {
            let standard = request_params(
                r#"{
                    "langsmith_project": "p",
                    "dd_api_key": "secret",
                    "gcs_bucket_name": "bucket",
                    "langfuse_host": "https://example.invalid",
                    "not_a_callback_field": "x"
                }"#,
                &TrustedCallbacks::default(),
                false,
            )
            .unwrap();
            assert_eq!(
                standard,
                IndexMap::from([("langsmith_project".to_owned(), text("p"))])
            );
        }
        Scenario::ParamsApplyTrustedOverlay => {
            let team = trusted(&[
                ("newrelic_api_key", "key"),
                ("newrelic_region", "eu"),
                ("signoz_ingestion_key", "signoz"),
                ("other", "x"),
            ]);
            let standard = request_params(
                r#"{"newrelic_region": "request", "langsmith_project": "p"}"#,
                &team,
                false,
            )
            .unwrap();
            assert_eq!(standard.get("newrelic_api_key"), Some(&text("key")));
            assert_eq!(standard.get("newrelic_region"), Some(&text("eu")));
            assert_eq!(standard.get("signoz_ingestion_key"), Some(&text("signoz")));
            assert_eq!(standard.get("other"), None);
        }
        Scenario::ParamsInheritedPrivacyWins => {
            let standard = request_params(
                r#"{"turn_off_message_logging": false, "langsmith_project": "p"}"#,
                &TrustedCallbacks::default(),
                true,
            )
            .unwrap();
            assert_eq!(
                standard.get("turn_off_message_logging"),
                Some(&Json::Bool(true))
            );
        }
        Scenario::ParamsRejectEnvironmentReferences => {
            let error = request_params(
                r#"{"langsmith_project": "os.environ/PRIVATE"}"#,
                &TrustedCallbacks::default(),
                false,
            )
            .unwrap_err();
            assert!(matches!(
                &error,
                DynamicCallbackError::Params(CallbackMetadataError::EnvironmentReference {
                    field
                }) if field == "langsmith_project"
            ));
            assert!(error.to_string().contains("langsmith_project"));
        }
        Scenario::ParamsRejectAShadowedEnvironmentReference => {
            let error = request_params(
                r#"{"langsmith_project": "direct", "metadata": {"langsmith_project": "os.environ/X"}}"#,
                &TrustedCallbacks::default(),
                false,
            )
            .unwrap_err();
            assert!(matches!(
                &error,
                DynamicCallbackError::Params(CallbackMetadataError::EnvironmentReference {
                    field
                }) if field == "langsmith_project"
            ));
        }
        Scenario::ParamsRejectAnInvalidLangfuseEnvironment => {
            let error = request_params(
                r#"{"langfuse_environment": "Production"}"#,
                &TrustedCallbacks::default(),
                false,
            )
            .unwrap_err();
            assert!(matches!(
                &error,
                DynamicCallbackError::Params(CallbackMetadataError::InvalidField { field })
                    if field == "langfuse_environment"
            ));
        }
        Scenario::ParamsEmptyRequestIsEmpty => {
            let standard = request_params("{}", &TrustedCallbacks::default(), false).unwrap();
            assert!(standard.is_empty());
        }
        Scenario::WireNamesConvertToListEntries => {
            let wire = DynamicCallbacks {
                input: Some(vec!["langfuse".to_owned()]),
                success: Some(Vec::new()),
                async_success: None,
                failure: None,
                async_failure: Some(vec!["otel".to_owned(), "s3_v2".to_owned()]),
            };
            let lists = DynamicCallbackLists::from(&wire);
            assert_eq!(labels(lists.input.as_deref().unwrap()), ["langfuse"]);
            assert_eq!(lists.success, Some(Vec::new()));
            assert!(lists.async_success.is_none());
            assert!(lists.failure.is_none());
            assert_eq!(
                labels(lists.async_failure.as_deref().unwrap()),
                ["otel", "s3_v2"]
            );
        }
    }
}
