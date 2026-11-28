#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::{
    DEFAULT_BLOCK_SECONDS, DEFAULT_SOURCE_LIMIT, DEFAULT_WINDOW_SECONDS, DISABLE_ENV,
    OverrideLimit, ProxyTopology, Raw, SourceOverride, SourceOverrides,
};
use crate::loader::{ConfigLoader, EnvironmentLookup as _, parse_view};
use crate::sections::Section as _;
use crate::sections::login_throttle::Settings;

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

fn loaded(source: &str) -> Settings {
    ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .login_throttle
}

fn converted(source: &str, environment: &BTreeMap<String, String>) -> Settings {
    let (raw, _ignored) = parse_view::<Raw>(source).unwrap();
    raw.convert(environment).unwrap()
}

#[test]
fn positive_integer_settings() {
    let configured = loaded(
        "general_settings:\n  max_failed_login_attempts_per_source: 20\n  failed_login_window_seconds: \" 45 \"\n  failed_login_block_seconds: 90\n",
    );
    assert_eq!(
        (
            configured.source_limit,
            configured.window_seconds,
            configured.block_seconds
        ),
        (20, 45, 90)
    );

    // int() accepts groups of digits joined by one underscore
    let underscored =
        loaded("general_settings:\n  max_failed_login_attempts_per_source: \"1_2\"\n");
    assert_eq!(underscored.source_limit, 12);
    assert_eq!(underscored.window_seconds, DEFAULT_WINDOW_SECONDS);
    assert_eq!(underscored.block_seconds, DEFAULT_BLOCK_SECONDS);

    let absent = loaded("general_settings: {}\n");
    assert_eq!(absent, Settings::default());
}

#[test]
fn null_setting_silent() {
    let source = "general_settings:\n  max_failed_login_attempts_per_source: null\n  failed_login_window_seconds: null\n  failed_login_block_seconds: null\n";
    let (configured, output) = logged(|| loaded(source));
    assert_eq!(configured, Settings::default());
    assert!(
        !output.contains(super::SOURCE_LIMIT_KEY)
            && !output.contains(super::WINDOW_KEY)
            && !output.contains(super::BLOCK_KEY),
        "{output}"
    );
}

#[test]
fn invalid_integer_warns() {
    let source = "general_settings:\n  max_failed_login_attempts_per_source: true\n  failed_login_window_seconds: \"soon\"\n  failed_login_block_seconds: 0\n";
    let (configured, output) = logged(|| loaded(source));
    assert_eq!(configured.source_limit, DEFAULT_SOURCE_LIMIT);
    assert_eq!(configured.window_seconds, DEFAULT_WINDOW_SECONDS);
    assert_eq!(configured.block_seconds, DEFAULT_BLOCK_SECONDS);
    assert_eq!(
        output.matches(super::SOURCE_LIMIT_KEY).count(),
        1,
        "{output}"
    );
    assert_eq!(output.matches(super::WINDOW_KEY).count(), 1, "{output}");
    assert_eq!(output.matches(super::BLOCK_KEY).count(), 1, "{output}");

    // a fractional float, a negative integer and a sign with no digits all fall back too
    let (fractional, fractional_output) = logged(|| {
        loaded(
            "general_settings:\n  max_failed_login_attempts_per_source: 1.5\n  failed_login_window_seconds: -3\n  failed_login_block_seconds: \"+\"\n",
        )
    });
    assert_eq!(fractional, Settings::default());
    assert_eq!(
        fractional_output.matches(super::SOURCE_LIMIT_KEY).count(),
        1,
        "{fractional_output}"
    );
    assert_eq!(
        fractional_output.matches(super::WINDOW_KEY).count(),
        1,
        "{fractional_output}"
    );
    assert_eq!(
        fractional_output.matches(super::BLOCK_KEY).count(),
        1,
        "{fractional_output}"
    );

    // int() rejects two underscores in a row, so the digits never join across the empty group
    let (underscored, underscored_output) =
        logged(|| loaded("general_settings:\n  max_failed_login_attempts_per_source: \"1__2\"\n"));
    assert_eq!(underscored.source_limit, DEFAULT_SOURCE_LIMIT);
    assert_eq!(
        underscored_output.matches(super::SOURCE_LIMIT_KEY).count(),
        1,
        "{underscored_output}"
    );
}

#[test]
fn exact_zero_override() {
    let configured = loaded(
        "general_settings:\n  max_failed_login_attempts_per_source_overrides:\n    \"10.0.0.1\": 0\n    \"10.0.0.2\": \" 0 \"\n    \"10.0.0.3\": 0.0\n    \"10.0.0.4\": false\n    \"10.0.0.5\": \"10\"\n    \"10.0.0.6\": \"00\"\n",
    );
    let expected = SourceOverrides::Mapping(vec![
        SourceOverride {
            range: "10.0.0.1".to_owned(),
            limit: OverrideLimit::Exempt,
        },
        SourceOverride {
            range: "10.0.0.2".to_owned(),
            limit: OverrideLimit::Exempt,
        },
        SourceOverride {
            range: "10.0.0.3".to_owned(),
            limit: OverrideLimit::Invalid,
        },
        SourceOverride {
            range: "10.0.0.4".to_owned(),
            limit: OverrideLimit::Invalid,
        },
        SourceOverride {
            range: "10.0.0.5".to_owned(),
            limit: OverrideLimit::Limit(10),
        },
        SourceOverride {
            range: "10.0.0.6".to_owned(),
            limit: OverrideLimit::Invalid,
        },
    ]);
    assert_eq!(configured.source_limit_overrides, expected);
}

#[test]
fn topology_absent_vs_empty_list() {
    assert_eq!(
        loaded("general_settings: {}\n").proxy_topology,
        ProxyTopology::Unknown
    );
    assert_eq!(
        loaded("general_settings:\n  trusted_proxy_ranges: null\n").proxy_topology,
        ProxyTopology::Unknown
    );
    assert_eq!(
        loaded("general_settings:\n  trusted_proxy_ranges: []\n").proxy_topology,
        ProxyTopology::Entries(Vec::new())
    );
}

#[test]
fn topology_wrong_shape() {
    let source = "general_settings:\n  trusted_proxy_ranges: true\n";
    let (settings, output) = logged(|| {
        let (raw, _ignored) = parse_view::<Raw>(source).unwrap();
        raw.convert(&BTreeMap::<String, String>::new()).unwrap()
    });
    assert_eq!(settings.proxy_topology, ProxyTopology::Unknown);
    assert_eq!(output.matches("WARN").count(), 1, "{output}");
    assert!(output.contains(super::TRUSTED_PROXY_RANGES_KEY), "{output}");
}

#[test]
fn topology_strings_are_split_and_iterables_are_stringified() {
    let comma = converted(
        "general_settings:\n  trusted_proxy_ranges: \" 10.0.0.0/8 , ,192.168.0.0/16 \"\n",
        &BTreeMap::new(),
    );
    assert_eq!(
        comma.proxy_topology,
        ProxyTopology::Entries(vec![
            "10.0.0.0/8".to_owned(),
            String::new(),
            "192.168.0.0/16".to_owned(),
        ])
    );
    let iterable = converted(
        "general_settings:\n  trusted_proxy_ranges:\n    - 10.0.0.0/8\n    - true\n",
        &BTreeMap::new(),
    );
    assert_eq!(
        iterable.proxy_topology,
        ProxyTopology::Entries(vec!["10.0.0.0/8".to_owned(), "True".to_owned()])
    );
}

#[test]
fn override_shapes() {
    assert_eq!(
        converted("general_settings: {}\n", &BTreeMap::new()).source_limit_overrides,
        SourceOverrides::Absent
    );
    assert_eq!(
        converted(
            "general_settings:\n  max_failed_login_attempts_per_source_overrides: null\n",
            &BTreeMap::new()
        )
        .source_limit_overrides,
        SourceOverrides::Absent
    );
    assert_eq!(
        converted(
            "general_settings:\n  max_failed_login_attempts_per_source_overrides: [\"10.0.0.0/8\"]\n",
            &BTreeMap::new()
        )
        .source_limit_overrides,
        SourceOverrides::Invalid
    );
}

#[test]
fn environment_references_resolve_and_a_missing_one_reads_as_absent() {
    let environment: BTreeMap<String, String> = [
        ("LIMIT".to_owned(), "7".to_owned()),
        ("WINDOW".to_owned(), "30".to_owned()),
    ]
    .into_iter()
    .collect();
    let (configured, output) = logged(|| {
        converted(
            "general_settings:\n  max_failed_login_attempts_per_source: os.environ/LIMIT\n  failed_login_window_seconds: os.environ/WINDOW\n  failed_login_block_seconds: os.environ/ABSENT\n",
            &environment,
        )
    });
    assert_eq!(
        (
            configured.source_limit,
            configured.window_seconds,
            configured.block_seconds
        ),
        (7, 30, DEFAULT_BLOCK_SECONDS)
    );
    assert_eq!(environment.lookup("ABSENT"), None);
    assert!(
        !output.contains(super::SOURCE_LIMIT_KEY)
            && !output.contains(super::WINDOW_KEY)
            && !output.contains(super::BLOCK_KEY),
        "{output}"
    );
}

#[test]
fn null_override_uses_the_source_default_silently() {
    let source = "general_settings:\n  max_failed_login_attempts_per_source: 6\n  max_failed_login_attempts_per_source_overrides:\n    \"10.0.0.0/8\": null\n";
    let (configured, output) = logged(|| loaded(source));
    assert_eq!(
        configured.source_limit_overrides,
        SourceOverrides::Mapping(vec![SourceOverride {
            range: "10.0.0.0/8".to_owned(),
            limit: OverrideLimit::Limit(6),
        }])
    );
    assert!(
        !output.contains(super::SOURCE_LIMIT_OVERRIDES_KEY),
        "{output}"
    );
}

#[test]
fn disabled_reads_the_trimmed_boolean_setting() {
    let enabled = |value: Option<&str>| {
        let environment: BTreeMap<String, String> = value
            .map(|text| BTreeMap::from([(DISABLE_ENV.to_owned(), text.to_owned())]))
            .unwrap_or_default();
        converted("general_settings: {}\n", &environment).disabled
    };
    assert!(enabled(Some(" TRUE ")));
    assert!(enabled(Some("true")));
    assert!(!enabled(Some("false")));
    assert!(!enabled(Some("yes")));
    assert!(!enabled(Some("")));
    assert!(!enabled(None));
}
