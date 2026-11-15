#![cfg(test)]

use std::collections::BTreeMap;

use super::{
    DEFAULT_MAX_DRAINS, DEFAULT_QUEUE_MAXSIZE, MAX_DETACHED_STREAM_DRAINS_ENV, QUEUE_MAXSIZE_ENV,
    Raw, Settings,
};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::Section as _;

fn settings(pairs: &[(&str, &str)]) -> Settings {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Raw::default()
        .convert(&environment)
        .expect("the fixture parses")
}

fn loader(pairs: &[(&str, &str)]) -> ConfigLoader<BTreeMap<String, String>> {
    ConfigLoader {
        lookup: pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

#[test]
fn loads_relay_settings() {
    assert_defaults_and_parsing();
    assert_invalid_text();
    assert_loader_reaches_the_section();
}

fn assert_defaults_and_parsing() {
    let unset = settings(&[]);
    assert_eq!(
        (unset.queue_maxsize, unset.max_detached_stream_drains),
        (DEFAULT_QUEUE_MAXSIZE, DEFAULT_MAX_DRAINS)
    );
    assert_eq!(
        (unset.queue_maxsize, unset.max_detached_stream_drains),
        (1024, 100)
    );
    let padded = settings(&[
        (QUEUE_MAXSIZE_ENV, " 2 "),
        (MAX_DETACHED_STREAM_DRAINS_ENV, "+1"),
    ]);
    assert_eq!(
        (padded.queue_maxsize, padded.max_detached_stream_drains),
        (2, 1)
    );
    let underscored = settings(&[(QUEUE_MAXSIZE_ENV, "1_000")]);
    assert_eq!(underscored.queue_maxsize, 1000);
    let blank = settings(&[
        (QUEUE_MAXSIZE_ENV, " "),
        (MAX_DETACHED_STREAM_DRAINS_ENV, ""),
    ]);
    assert_eq!(blank, Settings::default());
}

fn assert_invalid_text() {
    for disabled in ["0", "-1"] {
        let drains = settings(&[(MAX_DETACHED_STREAM_DRAINS_ENV, disabled)]);
        assert!(drains.max_detached_stream_drains <= 0, "{disabled}");
        let queue = settings(&[(QUEUE_MAXSIZE_ENV, disabled)]);
        assert!(queue.queue_maxsize <= 0, "{disabled}");
    }
    for invalid in ["bad", "1.5", "1__0", "_1", "1_", "9223372036854775808"] {
        let error = Raw::default()
            .convert(&BTreeMap::from([(
                QUEUE_MAXSIZE_ENV.to_owned(),
                invalid.to_owned(),
            )]))
            .expect_err("invalid text fails configuration");
        assert!(
            matches!(error, ConfigError::Environment { .. }),
            "{invalid}: {error}"
        );
    }
}

fn assert_loader_reaches_the_section() {
    let loaded = loader(&[(QUEUE_MAXSIZE_ENV, " 3 ")])
        .load("{}")
        .expect("the config loads");
    assert_eq!(loaded.sections.messages_native_stream.queue_maxsize, 3);
    let fallback = loader(&[])
        .load("model_list: []\n")
        .expect("the config loads");
    assert_eq!(
        fallback.sections.messages_native_stream,
        Settings::default()
    );
    let error = loader(&[(MAX_DETACHED_STREAM_DRAINS_ENV, "soon")])
        .load("{}")
        .unwrap_err();
    assert!(matches!(error, ConfigError::Environment { .. }), "{error}");
}
