use rstest::rstest;

use super::support::{Group, Single, capture_logs, dec, selected, spec, strategy_with, typed_pool};
use crate::lar1_routing::metadata::{Act, Evidence, Lar1Metadata, MetadataError, Mind, Time};
use crate::lar1_routing::strategy::{Lar1Error, Lar1Strategy, sync_deployment};
use crate::lar1_routing::thresholds::Thresholds;

#[derive(Clone, Copy)]
enum Behavior {
    DeclaredDefaults,
    WireEnums,
    ExactType,
    Fallback,
    UnverifiedFirst,
    MemorySecond,
    SingleDeployment,
    EmptyPool,
    NoRouter,
    InvalidMetadata,
    ExactMatchLog,
    FallbackLog,
    AsyncOnly,
    DefaultArgs,
}

#[rstest]
#[case::declared_defaults(Behavior::DeclaredDefaults)]
#[case::wire_enums(Behavior::WireEnums)]
#[case::exact_type(Behavior::ExactType)]
#[case::fallback(Behavior::Fallback)]
#[case::unverified_first(Behavior::UnverifiedFirst)]
#[case::memory_second(Behavior::MemorySecond)]
#[case::single_deployment(Behavior::SingleDeployment)]
#[case::empty_pool(Behavior::EmptyPool)]
#[case::no_router(Behavior::NoRouter)]
#[case::invalid_metadata(Behavior::InvalidMetadata)]
#[case::exact_match_log(Behavior::ExactMatchLog)]
#[case::fallback_log(Behavior::FallbackLog)]
#[case::async_only(Behavior::AsyncOnly)]
#[case::default_args(Behavior::DefaultArgs)]
#[tokio::test]
async fn lar1_routing_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::DeclaredDefaults => declared_defaults().await,
        Behavior::WireEnums => wire_enums().await,
        Behavior::ExactType => exact_type().await,
        Behavior::Fallback => fallback().await,
        Behavior::UnverifiedFirst => unverified_first().await,
        Behavior::MemorySecond => memory_second().await,
        Behavior::SingleDeployment => single_deployment().await,
        Behavior::EmptyPool => empty_pool().await,
        Behavior::NoRouter => no_router().await,
        Behavior::InvalidMetadata => invalid_metadata().await,
        Behavior::ExactMatchLog => exact_match_log().await,
        Behavior::FallbackLog => fallback_log().await,
        Behavior::AsyncOnly => async_only().await,
        Behavior::DefaultArgs => default_args().await,
    }
}

async fn declared_defaults() {
    let metadata = Lar1Metadata::parse(b"{}").unwrap();
    assert_eq!(metadata, Lar1Metadata::default());
    assert_eq!(metadata.act, Act::Inf);
    assert_eq!(metadata.time, Time::Now);
    assert_eq!(metadata.mind, Mind::Ref);
    assert_eq!(metadata.confidence, dec("0.5"));
    assert!(metadata.evidence.is_empty());
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(typed_pool())),
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("local-1"));
}

async fn wire_enums() {
    let metadata = Lar1Metadata::parse(
        br#"{"act":"GEN","time":"PRE","mind":"ACT","confidence":0.25,"evidence":["SYNTH","RETRIEVED","UNVERIFIED","CONFIRMED"],"ignored":true}"#,
    )
    .unwrap();
    assert_eq!(metadata.act, Act::Gen);
    assert_eq!(metadata.time, Time::Pre);
    assert_eq!(metadata.mind, Mind::Act);
    assert_eq!(metadata.confidence, dec("0.25"));
    assert_eq!(
        metadata.evidence,
        vec![
            Evidence::Synth,
            Evidence::Retrieved,
            Evidence::Unverified,
            Evidence::Confirmed
        ]
    );
    // a value outside the declared enum makes the metadata invalid rather than silently default
    assert!(matches!(
        Lar1Metadata::parse(br#"{"act":"gen"}"#),
        Err(MetadataError::Invalid { .. })
    ));
}

async fn exact_type() {
    // the declared type wins over deployment order, and the first deployment of that type wins
    let pool = vec![
        spec("deep-1", Some("deep")),
        spec("cloud-fast-1", Some("cloud-fast")),
        spec("cloud-fast-2", Some("cloud-fast")),
        spec("local-1", Some("local")),
    ];
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(pool)),
        Some(br#"{"confidence":0.4}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("cloud-fast-1"));
}

async fn fallback() {
    // no deployment of the classified type falls back to the first healthy deployment
    let pool = vec![
        spec("cloud-smart-1", Some("cloud-smart")),
        spec("deep-1", Some("deep")),
    ];
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(pool)),
        Some(br#"{"confidence":0.55}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("cloud-smart-1"));
}

async fn unverified_first() {
    // unverified evidence outranks both the memory time and the confidence thresholds
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(typed_pool())),
        Some(br#"{"confidence":0.99,"time":"MEM","evidence":["UNVERIFIED"]}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("cloud-smart-1"));
    // the evidence list is searched, not only its first entry, and the type matches a deployment
    // that is not the first healthy one
    let pool = vec![
        spec("local-1", Some("local")),
        spec("cloud-smart-1", Some("cloud-smart")),
    ];
    let contained = selected(
        &Lar1Strategy::default(),
        Some(&Group(pool)),
        Some(br#"{"confidence":0.99,"evidence":["SYNTH","UNVERIFIED"]}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(contained.as_deref(), Some("cloud-smart-1"));
}

async fn memory_second() {
    // a memory request is cloud-fast before any confidence threshold applies
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(typed_pool())),
        Some(br#"{"confidence":0.99,"time":"MEM"}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("cloud-fast-1"));
}

async fn single_deployment() {
    let (captured, _guard) = capture_logs();
    // a router that resolved one deployment returns it before classifying
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Single(spec("only-1", Some("deep")))),
        Some(br#"{"confidence":0.1}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("only-1"));
    assert!(captured.messages().is_empty());
}

async fn empty_pool() {
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(Vec::new())),
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(id, None);
}

async fn no_router() {
    let (captured, _guard) = capture_logs();
    // a strategy that holds no router answers None before it parses the metadata
    let id = selected(&Lar1Strategy::default(), None, Some(b"not json"), false)
        .await
        .unwrap();
    assert_eq!(id, None);
    assert!(captured.messages().is_empty());
}

async fn invalid_metadata() {
    let (captured, _guard) = capture_logs();
    let pool = vec![
        spec("cloud-smart-1", Some("cloud-smart")),
        spec("local-1", Some("local")),
    ];
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(pool.clone())),
        Some(br#"{"confidence":2}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("local-1"));
    assert!(captured.messages().contains("invalid lar1 metadata"));
    // malformed bytes take the same defaulting path
    let fallback_id = selected(
        &Lar1Strategy::default(),
        Some(&Group(pool)),
        Some(b"{"),
        false,
    )
    .await
    .unwrap();
    assert_eq!(fallback_id.as_deref(), Some("local-1"));
}

async fn exact_match_log() {
    let (captured, _guard) = capture_logs();
    let id = selected(
        &Lar1Strategy::default(),
        Some(&Group(typed_pool())),
        Some(br#"{"confidence":0.9}"#),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some("deep-1"));
    let messages = captured.messages();
    assert!(messages.contains("LAR-1 confidence routing"));
    assert!(messages.contains("confidence=0.9"));
    assert!(messages.contains("classified=deep"));
}

async fn fallback_log() {
    let (captured, _guard) = capture_logs();
    let pool = vec![spec("cloud-smart-1", Some("cloud-smart"))];
    let id = selected(&Lar1Strategy::default(), Some(&Group(pool)), None, false)
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("cloud-smart-1"));
    let messages = captured.messages();
    assert!(messages.contains("classified=local"));
    assert!(messages.contains("actual=cloud-smart"));
    // a deployment that declares no type reports "unknown" in the fallback warning
    let untyped = vec![spec("untyped-1", None)];
    let missing = selected(&Lar1Strategy::default(), Some(&Group(untyped)), None, false)
        .await
        .unwrap();
    assert_eq!(missing.as_deref(), Some("untyped-1"));
    assert!(captured.messages().contains("actual=unknown"));
}

async fn async_only() {
    let error = sync_deployment().unwrap_err();
    assert_eq!(error, Lar1Error::AsyncOnly);
    assert_eq!(
        error.to_string(),
        "LAR-1 routing only supports async routing. Enable async_only_mode on the router or use acompletion."
    );
}

async fn default_args() {
    assert_eq!(Thresholds::from_args(None).unwrap(), Thresholds::default());
    let strategy = strategy_with("0.2", "0.5", "0.8");
    assert_eq!(strategy.thresholds.low, dec("0.2"));
    assert_eq!(strategy.thresholds.medium, dec("0.5"));
    assert_eq!(strategy.thresholds.high, dec("0.8"));
}
