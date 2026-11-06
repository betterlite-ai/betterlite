#![cfg(test)]

use std::fs;
use std::io::{Result as IoResult, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use indexmap::IndexMap;
use rstest::rstest;
use tokio::sync::mpsc;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt::MakeWriter;

use super::catalog::{adapter_for, validate_callback_vars};
use super::error::RegistryError;
use super::identity::{Attribute, CallbackIdentity};
use super::phases::{CallbackList, CallbackPhase};
use super::sink::{CallbackEvent, CallbackSink, SinkError};
use super::{CallbackRegistry, Registration};
use crate::callback_config::error::ValidationError;
use crate::callback_config::{CallbackVars, FeatureFlags};

enum Scenario {
    RegistersInOrder,
    DuplicateKeepsFirst,
    DuplicateAfterCapacity,
    CapacityOverflow,
    CapacityPerList,
    DistinctAttributes,
    IdentityKey,
    Selection,
    StreamPhases,
    RemoveOneList,
    RemoveEveryList,
    RemoveClass,
    Delivery,
    FullChannel,
    ClosedChannel,
    TracingSink,
    KnownNames,
    UnknownName,
    ValidationKinds,
    ValidationPrecedence,
}

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for Buffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let result = with_default(subscriber, action);
    (result, buffer.contents())
}

fn named(class_name: &str) -> CallbackIdentity {
    CallbackIdentity {
        class_name: class_name.to_owned(),
        attributes: IndexMap::new(),
    }
}

fn attributed(class_name: &str, attributes: &[(&str, Attribute)]) -> CallbackIdentity {
    CallbackIdentity {
        class_name: class_name.to_owned(),
        attributes: attributes
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    }
}

fn vars(pairs: &[(&str, &str)]) -> CallbackVars {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn flags(otel_v2_enabled: bool) -> FeatureFlags {
    FeatureFlags { otel_v2_enabled }
}

fn channel_sink(capacity: usize) -> (CallbackSink, mpsc::Receiver<CallbackEvent>) {
    let (sender, receiver) = mpsc::channel(capacity);
    (CallbackSink::Channel(sender), receiver)
}

fn keys(registry: &CallbackRegistry, list: CallbackList) -> Vec<String> {
    registry
        .list_identities(list)
        .map(CallbackIdentity::key)
        .collect()
}

fn phase_keys(registry: &CallbackRegistry, phase: CallbackPhase) -> Vec<String> {
    registry
        .phase_identities(phase)
        .map(CallbackIdentity::key)
        .collect()
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[derive(serde::Deserialize)]
struct MetadataEntry {
    id: String,
}

#[rstest]
#[case::registers_each_list_in_order(Scenario::RegistersInOrder)]
#[case::duplicate_identity_keeps_the_first(Scenario::DuplicateKeepsFirst)]
#[case::capacity_is_checked_before_the_duplicate(Scenario::DuplicateAfterCapacity)]
#[case::capacity_overflow_skips_and_warns(Scenario::CapacityOverflow)]
#[case::capacity_is_counted_per_list(Scenario::CapacityPerList)]
#[case::distinct_attributes_are_distinct_identities(Scenario::DistinctAttributes)]
#[case::identity_key_matches_litellm(Scenario::IdentityKey)]
#[case::selection_splits_success_failure_and_both(Scenario::Selection)]
#[case::stream_phase_follows_the_success_lists(Scenario::StreamPhases)]
#[case::removal_takes_one_list(Scenario::RemoveOneList)]
#[case::removal_takes_every_list(Scenario::RemoveEveryList)]
#[case::removal_by_class_counts_each_membership(Scenario::RemoveClass)]
#[case::delivery_reaches_subscribed_sinks_only(Scenario::Delivery)]
#[case::full_channel_reports_the_sink_error(Scenario::FullChannel)]
#[case::closed_channel_reports_the_sink_error(Scenario::ClosedChannel)]
#[case::tracing_sink_logs_the_phase(Scenario::TracingSink)]
#[case::known_metadata_names_are_unavailable(Scenario::KnownNames)]
#[case::unknown_name_is_unavailable(Scenario::UnknownName)]
#[case::validators_report_each_kind(Scenario::ValidationKinds)]
#[case::validation_precedes_adapter_resolution(Scenario::ValidationPrecedence)]
fn callback_registry_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::RegistersInOrder => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(4);
            assert_eq!(
                registry.register(named("general"), sink.clone(), CallbackList::Callbacks),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("succ"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("fail"), sink.clone(), CallbackList::Failure),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("input"), sink.clone(), CallbackList::Input),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("service"), sink, CallbackList::Service),
                Registration::Registered
            );
            assert_eq!(keys(&registry, CallbackList::Callbacks), ["general"]);
            assert_eq!(keys(&registry, CallbackList::Success), ["succ"]);
            assert_eq!(keys(&registry, CallbackList::Failure), ["fail"]);
            assert_eq!(keys(&registry, CallbackList::Input), ["input"]);
            assert_eq!(keys(&registry, CallbackList::Service), ["service"]);
            assert_eq!(phase_keys(&registry, CallbackPhase::Input), ["input"]);
            assert_eq!(
                phase_keys(&registry, CallbackPhase::Success),
                ["general", "succ"]
            );
            assert_eq!(
                phase_keys(&registry, CallbackPhase::Failure),
                ["general", "fail"]
            );
            assert_eq!(
                phase_keys(&registry, CallbackPhase::Stream),
                ["general", "succ"]
            );
            assert_eq!(phase_keys(&registry, CallbackPhase::Service), ["service"]);
            let selection = registry.select();
            assert_eq!(selection.success, ["succ"]);
            assert_eq!(selection.failure, ["fail"]);
            assert_eq!(selection.success_and_failure, ["general"]);
        }
        Scenario::DuplicateKeepsFirst => {
            let mut registry = CallbackRegistry::new(100);
            let (first, mut receiver) = channel_sink(4);
            let (second, mut second_receiver) = channel_sink(4);
            let identity = attributed("Logger", &[("api_key", Attribute::Text("k".to_owned()))]);
            assert_eq!(
                registry.register(identity.clone(), first, CallbackList::Success),
                Registration::Registered
            );
            let (outcome, log) = logged(|| {
                registry.register(identity.clone(), second.clone(), CallbackList::Success)
            });
            assert_eq!(outcome, Registration::AlreadyRegistered);
            assert!(log.contains("callback already registered in this list"));
            assert_eq!(
                registry.register(identity, second, CallbackList::Callbacks),
                Registration::Registered
            );
            assert_eq!(keys(&registry, CallbackList::Success), ["Logger-api_key=k"]);
            assert_eq!(
                keys(&registry, CallbackList::Callbacks),
                ["Logger-api_key=k"]
            );
            registry.deliver(&CallbackEvent {
                phase: CallbackPhase::Success,
            });
            receiver.try_recv().unwrap();
            second_receiver.try_recv().unwrap_err();
        }
        Scenario::DuplicateAfterCapacity => {
            let mut registry = CallbackRegistry::new(1);
            let (sink, _receiver) = channel_sink(4);
            let identity = named("Only");
            assert_eq!(
                registry.register(identity.clone(), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            let (outcome, log) =
                logged(|| registry.register(identity, sink, CallbackList::Success));
            assert_eq!(outcome, Registration::SkippedCapacity);
            assert!(log.contains("callback skipped: the list is at capacity"));
        }
        Scenario::CapacityOverflow => {
            let mut registry = CallbackRegistry::new(2);
            let (sink, _receiver) = channel_sink(4);
            assert_eq!(
                registry.register(named("first"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("second"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            let (outcome, log) =
                logged(|| registry.register(named("third"), sink, CallbackList::Success));
            assert_eq!(outcome, Registration::SkippedCapacity);
            assert!(log.contains("callback skipped: the list is at capacity"));
            assert_eq!(keys(&registry, CallbackList::Success), ["first", "second"]);
        }
        Scenario::CapacityPerList => {
            let mut registry = CallbackRegistry::new(1);
            let (sink, _receiver) = channel_sink(4);
            assert_eq!(
                registry.register(named("first"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("second"), sink, CallbackList::Callbacks),
                Registration::Registered
            );
        }
        Scenario::DistinctAttributes => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(4);
            let first = attributed("Logger", &[("region", Attribute::Text("us".to_owned()))]);
            let second = attributed("Logger", &[("region", Attribute::Text("eu".to_owned()))]);
            assert_eq!(
                registry.register(first, sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(second, sink, CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                keys(&registry, CallbackList::Success),
                ["Logger-region=us", "Logger-region=eu"]
            );
        }
        Scenario::IdentityKey => {
            let identity = attributed(
                "MyLogger",
                &[
                    ("api_key", Attribute::Text("sk-1".to_owned())),
                    ("verbose", Attribute::Flag(true)),
                    ("retries", Attribute::Integer(3)),
                ],
            );
            assert_eq!(
                identity.key(),
                "MyLogger-api_key=sk-1-verbose=True-retries=3"
            );
            assert_eq!(named("langfuse").key(), "langfuse");
        }
        Scenario::Selection => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(8);
            assert_eq!(
                registry.register(named("both"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("both"), sink.clone(), CallbackList::Failure),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("succ"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("fail"), sink.clone(), CallbackList::Failure),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("general"), sink, CallbackList::Callbacks),
                Registration::Registered
            );
            let selection = registry.select();
            assert_eq!(selection.success, ["succ"]);
            assert_eq!(selection.failure, ["fail"]);
            assert_eq!(
                selection.success_and_failure,
                ["both", "general"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<String>>()
            );
        }
        Scenario::StreamPhases => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(4);
            assert_eq!(
                registry.register(named("succ"), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("fail"), sink, CallbackList::Failure),
                Registration::Registered
            );
            assert_eq!(phase_keys(&registry, CallbackPhase::Stream), ["succ"]);
            assert_eq!(phase_keys(&registry, CallbackPhase::Failure), ["fail"]);
        }
        Scenario::RemoveOneList => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(4);
            let identity = named("Logger");
            assert_eq!(
                registry.register(identity.clone(), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(identity.clone(), sink, CallbackList::Callbacks),
                Registration::Registered
            );
            assert!(registry.remove_from_list(&identity, CallbackList::Success));
            assert!(keys(&registry, CallbackList::Success).is_empty());
            assert_eq!(keys(&registry, CallbackList::Callbacks), ["Logger"]);
            assert_eq!(phase_keys(&registry, CallbackPhase::Success), ["Logger"]);
            assert!(!registry.remove_from_list(&identity, CallbackList::Success));
        }
        Scenario::RemoveEveryList => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(4);
            let identity = named("Logger");
            assert_eq!(
                registry.register(identity.clone(), sink.clone(), CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(identity.clone(), sink, CallbackList::Callbacks),
                Registration::Registered
            );
            assert!(registry.remove_from_list(&identity, CallbackList::Success));
            assert!(registry.remove_from_list(&identity, CallbackList::Callbacks));
            for phase in CallbackPhase::ALL {
                assert!(phase_keys(&registry, phase).is_empty());
            }
            assert_eq!(
                registry.register(named("Logger"), channel_sink(1).0, CallbackList::Success),
                Registration::Registered
            );
        }
        Scenario::RemoveClass => {
            let mut registry = CallbackRegistry::new(100);
            let (sink, _receiver) = channel_sink(8);
            for identity in [
                attributed("Kept", &[("id", Attribute::Integer(1))]),
                attributed("Gone", &[("id", Attribute::Integer(2))]),
                attributed("Gone", &[("id", Attribute::Integer(3))]),
            ] {
                assert_eq!(
                    registry.register(identity, sink.clone(), CallbackList::Success),
                    Registration::Registered
                );
            }
            assert_eq!(
                registry.remove_class_from_list("Gone", CallbackList::Success),
                2
            );
            assert_eq!(keys(&registry, CallbackList::Success), ["Kept-id=1"]);
            assert_eq!(
                registry.remove_class_from_list("Absent", CallbackList::Success),
                0
            );
        }
        Scenario::Delivery => {
            let mut registry = CallbackRegistry::new(100);
            let (success_sink, mut success_receiver) = channel_sink(4);
            let (failure_sink, mut failure_receiver) = channel_sink(4);
            assert_eq!(
                registry.register(named("succ"), success_sink, CallbackList::Success),
                Registration::Registered
            );
            assert_eq!(
                registry.register(named("fail"), failure_sink, CallbackList::Failure),
                Registration::Registered
            );
            registry.deliver(&CallbackEvent {
                phase: CallbackPhase::Success,
            });
            assert_eq!(
                success_receiver.try_recv().unwrap().phase,
                CallbackPhase::Success
            );
            failure_receiver.try_recv().unwrap_err();
            registry.deliver(&CallbackEvent {
                phase: CallbackPhase::Service,
            });
            success_receiver.try_recv().unwrap_err();
        }
        Scenario::FullChannel => {
            let (sink, _receiver) = channel_sink(1);
            let event = CallbackEvent {
                phase: CallbackPhase::Success,
            };
            sink.deliver(&event).unwrap();
            assert!(matches!(sink.deliver(&event), Err(SinkError::Full)));
        }
        Scenario::ClosedChannel => {
            let (sink, receiver) = channel_sink(1);
            drop(receiver);
            let event = CallbackEvent {
                phase: CallbackPhase::Success,
            };
            assert!(matches!(sink.deliver(&event), Err(SinkError::Closed)));
        }
        Scenario::TracingSink => {
            let mut registry = CallbackRegistry::new(100);
            assert_eq!(
                registry.register(named("trace"), CallbackSink::Tracing, CallbackList::Success),
                Registration::Registered
            );
            let log = logged(|| {
                registry.deliver(&CallbackEvent {
                    phase: CallbackPhase::Success,
                });
            })
            .1;
            assert!(log.contains("callback event"));
            assert!(log.contains("success"));
        }
        Scenario::KnownNames => {
            let ids: Vec<MetadataEntry> = serde_json::from_str(
                &fs::read_to_string(fixture("callback_configs.json")).unwrap(),
            )
            .unwrap();
            let registry_names: Vec<String> = serde_json::from_str(
                &fs::read_to_string(fixture("callback_registry_names.json")).unwrap(),
            )
            .unwrap();
            let names: Vec<String> = ids
                .into_iter()
                .map(|entry| entry.id)
                .chain(registry_names)
                .collect();
            assert!(names.len() > 50);
            let mut registry = CallbackRegistry::new(100);
            for name in names {
                assert!(matches!(
                    adapter_for(&name),
                    Err(RegistryError::UnavailableAdapter { name: rejected }) if rejected == name
                ));
                assert!(matches!(
                    registry.register_configured(
                        &name,
                        &CallbackVars::new(),
                        CallbackList::Success,
                        flags(false)
                    ),
                    Err(RegistryError::UnavailableAdapter { .. })
                ));
            }
            assert!(keys(&registry, CallbackList::Success).is_empty());
        }
        Scenario::UnknownName => {
            assert!(matches!(
                adapter_for("definitely_not_a_litellm_callback"),
                Err(RegistryError::UnavailableAdapter { .. })
            ));
        }
        Scenario::ValidationKinds => {
            validate_callback_vars(Some("arize"), &vars(&[]), &[], flags(false)).unwrap();
            assert!(matches!(
                validate_callback_vars(
                    Some("arize"),
                    &vars(&[("arize_success_sampling_rate", "2")]),
                    &[],
                    flags(false)
                ),
                Err(ValidationError::SamplingRateOutOfRange { .. })
            ));
            assert!(matches!(
                validate_callback_vars(
                    Some("langfuse"),
                    &vars(&[("langfuse_secret_key", "t")]),
                    &[vars(&[("langfuse_secret", "s")])],
                    flags(false)
                ),
                Err(ValidationError::FamilyConflict { .. })
            ));
            assert!(matches!(
                validate_callback_vars(
                    Some("langfuse_otel"),
                    &vars(&[("langfuse_span_scope", "full")]),
                    &[vars(&[("langfuse_span_scope", "llm_only")])],
                    flags(false)
                ),
                Err(ValidationError::SpanScopeConflict { .. })
            ));
            assert!(matches!(
                validate_callback_vars(
                    Some("langfuse"),
                    &vars(&[("langfuse_span_scope", "full")]),
                    &[],
                    flags(false)
                ),
                Err(ValidationError::SpanScopeCallback { .. })
            ));
        }
        Scenario::ValidationPrecedence => {
            let mut registry = CallbackRegistry::new(100);
            let error = registry
                .register_configured(
                    "langfuse",
                    &vars(&[("langfuse_span_scope", "full")]),
                    CallbackList::Success,
                    flags(false),
                )
                .unwrap_err();
            assert!(matches!(
                error,
                RegistryError::Validation {
                    source: ValidationError::SpanScopeCallback { .. }
                }
            ));
            assert!(keys(&registry, CallbackList::Success).is_empty());
        }
    }
}
