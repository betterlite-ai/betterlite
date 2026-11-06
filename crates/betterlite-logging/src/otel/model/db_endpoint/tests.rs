#![cfg(test)]
use super::{DatabaseEndpoint, db_span_attributes, parse_database_endpoint, postgres_endpoint};
use betterlite_config::loader::EnvironmentLookup;
use rstest::rstest;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;
use std::sync::Mutex;

type Expected<'text> = Option<(Option<&'text str>, Option<i64>, Option<&'text str>)>;

fn endpoint(address: Option<&str>, port: Option<i64>, namespace: Option<&str>) -> DatabaseEndpoint {
    DatabaseEndpoint {
        address: address.map(str::to_owned),
        port,
        namespace: namespace.map(str::to_owned),
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(""), None)]
#[case::full(Some("postgresql://u:p@db.example.com:5433/app"), Some((Some("db.example.com"), Some(5433), Some("app"))))]
#[case::short_scheme(Some("postgres://u@db/app"), Some((Some("db"), Some(5432), Some("app"))))]
#[case::scheme_case(Some("POSTGRES://DB.Example/app"), Some((Some("db.example"), Some(5432), Some("app"))))]
#[case::leading_control_chars(Some("  \tpostgresql://h/db"), Some((Some("h"), Some(5432), Some("db"))))]
#[case::no_database(Some("postgresql://h"), Some((Some("h"), Some(5432), None)))]
#[case::schema_qualified(Some("postgresql://h/app?schema=tenant"), Some((Some("h"), Some(5432), Some("app|tenant"))))]
#[case::default_schema_implicit(Some("postgresql://h/app?schema=public"), Some((Some("h"), Some(5432), Some("app"))))]
#[case::schema_case_sensitive(Some("postgresql://h/app?schema=PUBLIC"), Some((Some("h"), Some(5432), Some("app|PUBLIC"))))]
#[case::schema_only(Some("postgresql://h?schema=tenant"), Some((Some("h"), Some(5432), Some("tenant"))))]
#[case::encoded_database(Some("postgresql://h/my%20db"), Some((Some("h"), Some(5432), Some("my db"))))]
#[case::unix_socket(Some("postgresql:///app?host=/var/run/postgresql"), Some((Some("/var/run/postgresql"), Some(5432), Some("app"))))]
#[case::encoded_socket(Some("postgresql://u:p@/app?host=%2Fcloudsql%2Fp%3Ar%3Ai"), Some((Some("/cloudsql/p:r:i"), Some(5432), Some("app"))))]
#[case::cloud_sql_host_beats_netloc(Some("postgresql://u:p@localhost/app?host=10.0.0.5&port=6543"), Some((Some("10.0.0.5"), Some(6543), Some("app"))))]
#[case::blank_host_ignored(Some("postgresql://h/app?host="), Some((Some("h"), Some(5432), Some("app"))))]
#[case::first_host_wins(Some("postgresql://h/app?host=a&host=b"), Some((Some("a"), Some(5432), Some("app"))))]
#[case::blank_first_host(Some("postgresql://h/app?host=&host=b"), Some((Some("b"), Some(5432), Some("app"))))]
#[case::encoded_query_key(Some("postgresql://h/app?ho%73t=q"), Some((Some("q"), Some(5432), Some("app"))))]
#[case::plus_is_space(Some("postgresql://h/app?host=a+b"), Some((Some("a b"), Some(5432), Some("app"))))]
#[case::query_port_beats_netloc(Some("postgresql://h:1/app?port=2"), Some((Some("h"), Some(2), Some("app"))))]
#[case::query_port_zero(Some("postgresql://h:1/app?port=0"), Some((Some("h"), Some(0), Some("app"))))]
#[case::query_port_not_digits(Some("postgresql://h:7/app?port=x"), Some((Some("h"), Some(7), Some("app"))))]
#[case::netloc_port_zero_defaults(Some("postgresql://h:0/app"), Some((Some("h"), Some(5432), Some("app"))))]
#[case::max_port(Some("postgresql://h:65535/app"), Some((Some("h"), Some(0xFFFF), Some("app"))))]
#[case::ipv6(Some("postgresql://u@[::1]:5433/app"), Some((Some("::1"), Some(5433), Some("app"))))]
#[case::ipv6_without_port(Some("postgresql://[2001:DB8::1]/app"), Some((Some("2001:db8::1"), Some(5432), Some("app"))))]
#[case::ipv6_zone_kept(Some("postgresql://[fe80::1%25eth0]/app"), Some((Some("fe80::1%25eth0"), Some(5432), Some("app"))))]
#[case::ipv6_zone_case_kept(Some("postgresql://[fe80::1%25ETH0]/app"), Some((Some("fe80::1%25ETH0"), Some(5432), Some("app"))))]
#[case::unclosed_bracket(Some("postgresql://[::1/app"), None)]
#[case::bad_ipv6(Some("postgresql://[nope]/app"), None)]
#[case::bad_port(Some("postgresql://h:abc/app"), None)]
#[case::port_out_of_range(Some("postgresql://h:70000/app"), None)]
#[case::bad_port_without_address(Some("postgresql://:abc/app"), Some((None, None, Some("app"))))]
#[case::not_postgres(Some("mysql://h/app"), None)]
#[case::redis(Some("redis://h:6379/0"), None)]
#[case::no_scheme(Some("h/app"), None)]
#[case::nothing_left(Some("postgresql://"), None)]
#[case::only_slash(Some("postgresql:///"), None)]
#[case::slash_in_password(Some("postgresql://user:pa/ss@host:5432/db"), None)]
#[case::hash_in_password(Some("postgresql://user:pa#ss@host/db"), None)]
#[case::question_in_password(Some("postgresql://user:pa?ss@host/db"), None)]
#[case::at_sign_in_query(Some("postgresql://h/app?application_name=svc@prod"), None)]
#[case::second_path_segment(Some("postgresql://h/a/b"), None)]
#[case::encoded_at_sign_in_query(Some("postgresql://h/app?application_name=svc%40prod"), Some((Some("h"), Some(5432), Some("app"))))]
fn database_endpoint_parses_dsn(#[case] url: Option<&str>, #[case] expected: Expected<'_>) {
    let parsed = parse_database_endpoint(url);
    assert_eq!(
        parsed,
        expected.map(|(address, port, namespace)| endpoint(address, port, namespace))
    );
}

#[rstest]
fn replays_the_database_endpoint_corpus(
    #[files("fuzz/corpus/parse_database_endpoint/*")] path: PathBuf,
) {
    let data = fs::read(&path).expect("corpus file");
    let Ok(text) = str::from_utf8(&data) else {
        return;
    };
    drop(parse_database_endpoint(Some(text)));
}

#[rstest]
#[case::primary(&[("DATABASE_URL", "postgresql://p/app")], Some("p"))]
#[case::replica_configured(&[("DATABASE_URL", "postgresql://p/app"), ("DATABASE_URL_READ_REPLICA", "postgresql://r/app")], None)]
#[case::blank_replica_ignored(&[("DATABASE_URL", "postgresql://p/app"), ("DATABASE_URL_READ_REPLICA", "")], Some("p"))]
#[case::missing_url(&[], None)]
#[case::blank_url(&[("DATABASE_URL", "")], None)]
fn postgres_endpoint_reads_environment(
    #[case] pairs: &[(&str, &str)],
    #[case] address: Option<&str>,
) {
    let found = postgres_endpoint(&environment(pairs));
    assert_eq!(found.and_then(|parsed| parsed.address).as_deref(), address);
}

struct Rotating(Mutex<BTreeMap<String, String>>);

impl EnvironmentLookup for Rotating {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.lock().ok()?.get(name).cloned()
    }
}

#[test]
fn postgres_endpoint_follows_rotation_without_caching() {
    let rotating = Rotating(Mutex::new(environment(&[(
        "DATABASE_URL",
        "postgresql://old:5432/app",
    )])));
    let before = postgres_endpoint(&rotating).and_then(|found| found.address);
    rotating.0.lock().expect("lock").insert(
        "DATABASE_URL".to_owned(),
        "postgresql://new:5432/app".to_owned(),
    );
    let after = postgres_endpoint(&rotating).and_then(|found| found.address);
    assert_eq!(
        (before.as_deref(), after.as_deref()),
        (Some("old"), Some("new"))
    );
}

fn attributes(
    service: &str,
    call_type: Option<&str>,
    pairs: &[(&str, &str)],
) -> Vec<(String, String)> {
    db_span_attributes(service, call_type, &environment(pairs))
        .into_iter()
        .map(|attribute| (attribute.key.to_string(), attribute.value.to_string()))
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::postgres("postgres", Some("select"), &[("DATABASE_URL", "postgresql://u:p@db:5433/app?schema=t")], &[("db.system.name", "postgresql"), ("db.system", "postgresql"), ("db.operation.name", "select"), ("server.address", "db"), ("server.port", "5433"), ("db.namespace", "app|t")])]
#[case::batch_write("batch_write_to_db", None, &[("DATABASE_URL", "postgresql://db/app")], &[("db.system.name", "postgresql"), ("db.system", "postgresql"), ("server.address", "db"), ("server.port", "5432"), ("db.namespace", "app")])]
#[case::replica_drops_endpoint("postgres", Some("select"), &[("DATABASE_URL", "postgresql://db/app"), ("DATABASE_URL_READ_REPLICA", "postgresql://r/app")], &[("db.system.name", "postgresql"), ("db.system", "postgresql"), ("db.operation.name", "select")])]
#[case::malformed_url("postgres", None, &[("DATABASE_URL", "mysql://db/app")], &[("db.system.name", "postgresql"), ("db.system", "postgresql")])]
#[case::port_zero_kept("postgres", None, &[("DATABASE_URL", "postgresql://db/app?port=0")], &[("db.system.name", "postgresql"), ("db.system", "postgresql"), ("server.address", "db"), ("server.port", "0"), ("db.namespace", "app")])]
#[case::empty_call_type_kept("postgres", Some(""), &[], &[("db.system.name", "postgresql"), ("db.system", "postgresql"), ("db.operation.name", "")])]
#[case::redis_has_no_endpoint("redis", Some("get"), &[("DATABASE_URL", "postgresql://db/app")], &[("db.system.name", "redis"), ("db.system", "redis"), ("db.operation.name", "get")])]
#[case::redis_queue("redis_daily_spend_update_queue", None, &[], &[("db.system.name", "redis"), ("db.system", "redis")])]
#[case::not_a_datastore("auth", Some("x"), &[("DATABASE_URL", "postgresql://db/app")], &[])]
fn database_span_attributes_name_the_datastore(
    #[case] service: &str,
    #[case] call_type: Option<&str>,
    #[case] environment: &[(&str, &str)],
    #[case] expected: &[(&str, &str)],
) {
    assert_eq!(attributes(service, call_type, environment), pairs(expected));
}
