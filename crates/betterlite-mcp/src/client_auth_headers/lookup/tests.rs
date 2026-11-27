#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::{lookup_server_auth, sanitize_alias_for_header};
use crate::client_auth_headers::tests::support::{exposed, groups, headers};

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<BTreeMap<String, String>>>>);

impl Logs {
    fn events(&self) -> Vec<BTreeMap<String, String>> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct Fields(BTreeMap<String, String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
}

fn assert_ambiguity_logged(logs: &Logs, alias: Option<&str>, access_groups: &[&str]) {
    let events = logs.events();
    assert_eq!(events.len(), 1);
    let event = events.first().unwrap();
    let expected = format!(
        "Ambiguous MCP group auth headers for server alias={} (groups={:?}); not forwarding any group credential",
        alias.unwrap_or_default(),
        groups(access_groups)
    );
    assert_eq!(
        event.get("message").map(String::as_str),
        Some(expected.as_str())
    );
    for value in event.values() {
        assert!(!value.contains("test-"), "{value}");
    }
}

#[rstest]
#[case::mixed_case("MyServer", "myserver")]
#[case::inner_space("my server", "my_server")]
#[case::outer_spaces("  my-server  ", "my_server")]
#[case::dash("my-server", "my_server")]
#[case::dots("my.server", "my_server")]
#[case::underscore_run("my___server", "my_server")]
#[case::leading_and_trailing_symbols("--my-server!!", "my_server")]
#[case::empty_result("!!!", "")]
#[case::kelvin_sign("\u{212a}elvin", "kelvin")]
#[case::dotted_capital_i("\u{130}", "i")]
fn sanitize_alias(#[case] alias: &str, #[case] expected: &str) {
    assert_eq!(sanitize_alias_for_header(alias), expected);
}

#[derive(Clone, Copy, Debug)]
enum Expected {
    Headers(&'static str),
    None,
    Ambiguous,
}

#[rstest]
#[case::empty_map(&[], None, None, &[], Expected::None)]
#[case::lowercase_alias(
    &[("github", "Authorization", "Bearer test-github")],
    Some("GitHub"),
    None,
    &[],
    Expected::Headers("github")
)]
#[case::sanitized_alias(
    &[("my_server", "Authorization", "Bearer test-underscore")],
    Some("My Server"),
    None,
    &[],
    Expected::Headers("my_server")
)]
#[case::alias_before_server_name(
    &[
        ("alpha", "Authorization", "Bearer test-alpha"),
        ("beta", "Authorization", "Bearer test-beta")
    ],
    Some("Alpha"),
    Some("beta"),
    &[],
    Expected::Headers("alpha")
)]
#[case::empty_alias_skips_to_server_name(
    &[("beta", "Authorization", "Bearer test-beta")],
    Some(""),
    Some("Beta"),
    &[],
    Expected::Headers("beta")
)]
#[case::lowercase_before_sanitized(
    &[
        ("my server", "Authorization", "Bearer test-raw"),
        ("my_server", "Authorization", "Bearer test-sanitized")
    ],
    Some("My Server"),
    None,
    &[],
    Expected::Headers("my server")
)]
#[case::uppercase_map_keys(
    &[("GitHub", "Authorization", "Bearer test-upper")],
    Some("github"),
    None,
    &[],
    Expected::Headers("GitHub")
)]
#[case::no_match(
    &[("github", "Authorization", "Bearer test-github")],
    Some("gitlab"),
    Some("gitlab"),
    &[],
    Expected::None
)]
#[case::empty_sanitized_alias(
    &[("", "Authorization", "Bearer test-empty")],
    Some("!!!"),
    None,
    &[],
    Expected::None
)]
fn server_key_order(
    #[case] entries: &[(&str, &str, &str)],
    #[case] alias: Option<&str>,
    #[case] server_name: Option<&str>,
    #[case] access_groups: &[&str],
    #[case] expected: Expected,
) {
    let built = headers(entries);
    let found = lookup_server_auth(&built, alias, server_name, &groups(access_groups));
    match expected {
        Expected::Headers(key) => {
            assert_eq!(found.map(exposed), Some(exposed(built.get(key).unwrap())))
        }
        Expected::None | Expected::Ambiguous => assert!(found.is_none()),
    }
}

#[rstest]
#[case::one_group(
    &[("team-a", "Authorization", "Bearer test-group")],
    None,
    &["team-a"],
    Expected::Headers("team-a")
)]
#[case::equal_groups(
    &[
        ("team-a", "Authorization", "Bearer test-equal"),
        ("team-b", "Authorization", "Bearer test-equal")
    ],
    None,
    &["team-a", "team-b"],
    Expected::Headers("team-a")
)]
#[case::different_groups(
    &[
        ("team-a", "Authorization", "Bearer test-a"),
        ("team-b", "Authorization", "Bearer test-b")
    ],
    Some("unrelated"),
    &["team-a", "team-b"],
    Expected::Ambiguous
)]
#[case::different_map_sizes(
    &[
        ("team-a", "Authorization", "Bearer test-a"),
        ("team-b", "Authorization", "Bearer test-a"),
        ("team-b", "X-Extra", "test-extra")
    ],
    None,
    &["team-a", "team-b"],
    Expected::Ambiguous
)]
#[case::missing_group_beside_a_match(
    &[("team-a", "Authorization", "Bearer test-group")],
    None,
    &["missing", "team-a"],
    Expected::Headers("team-a")
)]
#[case::empty_group_name(
    &[("", "Authorization", "Bearer test-empty")],
    None,
    &[""],
    Expected::Headers("")
)]
#[case::server_key_before_group(
    &[
        ("github", "Authorization", "Bearer test-github"),
        ("team-a", "Authorization", "Bearer test-group")
    ],
    Some("github"),
    &["team-a"],
    Expected::Headers("github")
)]
fn group_default(
    #[case] entries: &[(&str, &str, &str)],
    #[case] alias: Option<&str>,
    #[case] access_groups: &[&str],
    #[case] expected: Expected,
) {
    let logs = Logs::default();
    let _guard = set_default(registry().with(logs.clone()));
    let built = headers(entries);
    let found = lookup_server_auth(&built, alias, None, &groups(access_groups));
    match expected {
        Expected::Headers(key) => {
            assert_eq!(found.map(exposed), Some(exposed(built.get(key).unwrap())));
            assert!(logs.events().is_empty());
        }
        Expected::None => {
            assert!(found.is_none());
            assert!(logs.events().is_empty());
        }
        Expected::Ambiguous => {
            assert!(found.is_none());
            assert_ambiguity_logged(&logs, alias, access_groups);
        }
    }
}
