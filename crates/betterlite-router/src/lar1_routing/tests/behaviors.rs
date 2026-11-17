use rstest::rstest;

use super::support::{Group, Single, args, capture_logs, dec, selected, spec, typed_pool};
use crate::lar1_routing::metadata::{Act, Evidence, Lar1Metadata, MetadataError, Mind, Time};
use crate::lar1_routing::strategy::{Lar1Error, Lar1Strategy, sync_deployment};
use crate::lar1_routing::thresholds::Thresholds;

#[derive(Clone, Copy)]
enum Behavior {
    DeclaredDefaults,
    WireEnums,
    Selection {
        pool: &'static [(&'static str, &'static str)],
        metadata: &'static [u8],
        expected: &'static str,
    },
    UnverifiedFirst,
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
// the declared type wins over deployment order, and the first deployment of that type wins
#[case::exact_type(Behavior::Selection {
    pool: &[
        ("deep-1", "deep"),
        ("cloud-fast-1", "cloud-fast"),
        ("cloud-fast-2", "cloud-fast"),
        ("local-1", "local"),
    ],
    metadata: br#"{"confidence":0.4}"#,
    expected: "cloud-fast-1",
})]
// no deployment of the classified type falls back to the first healthy deployment
#[case::fallback(Behavior::Selection {
    pool: &[("cloud-smart-1", "cloud-smart"), ("deep-1", "deep")],
    metadata: br#"{"confidence":0.55}"#,
    expected: "cloud-smart-1",
})]
#[case::unverified_first(Behavior::UnverifiedFirst)]
// a memory request is cloud-fast before any confidence threshold applies
#[case::memory_second(Behavior::Selection {
    pool: &[
        ("cloud-smart-1", "cloud-smart"),
        ("cloud-fast-1", "cloud-fast"),
        ("local-1", "local"),
        ("deep-1", "deep"),
    ],
    metadata: br#"{"confidence":0.99,"time":"MEM"}"#,
    expected: "cloud-fast-1",
})]
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
        Behavior::Selection {
            pool,
            metadata,
            expected,
        } => {
            let deployments = pool
                .iter()
                .map(|&(id, kind)| spec(id, Some(kind)))
                .collect();
            let id = selected(
                &Lar1Strategy::default(),
                Some(&Group(deployments)),
                Some(metadata),
                false,
            )
            .await
            .unwrap();
            assert_eq!(id.as_deref(), Some(expected));
        }
        Behavior::UnverifiedFirst => unverified_first().await,
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
    // the evidence list is searched, and the type matches a deployment that is not first
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
    let defaults = Thresholds::default();
    assert_eq!(defaults.low, dec("0.3"));
    assert_eq!(defaults.medium, dec("0.5"));
    assert_eq!(defaults.high, dec("0.7"));
    // a missing argument keeps its own default while the configured ones are used
    let merged = Lar1Strategy::new(Some(&args(Some("0.2"), None, None))).unwrap();
    assert_eq!(merged.thresholds.low, dec("0.2"));
    assert_eq!(merged.thresholds.medium, dec("0.5"));
    assert_eq!(merged.thresholds.high, dec("0.7"));
    let strategy = Lar1Strategy::new(Some(&args(Some("0.2"), Some("0.5"), Some("0.8")))).unwrap();
    assert_eq!(strategy.thresholds.low, dec("0.2"));
    assert_eq!(strategy.thresholds.medium, dec("0.5"));
    assert_eq!(strategy.thresholds.high, dec("0.8"));
}
