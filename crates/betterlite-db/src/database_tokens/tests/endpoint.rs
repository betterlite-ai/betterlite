#![cfg(test)]

use rstest::rstest;

use crate::database_tokens::endpoint::{IAMEndpoint, parse_iam_endpoint_from_url};

fn endpoint(user: &str, name: &str, schema: Option<&str>) -> IAMEndpoint {
    IAMEndpoint {
        host: "db.example".to_owned(),
        port: "5432".to_owned(),
        user: user.to_owned(),
        name: name.to_owned(),
        schema: schema.map(str::to_owned),
    }
}

#[test]
fn normalized_url_components() {
    // a raw UPN gets its '@' encoded, and an already encoded one is left byte for byte alone
    assert_eq!(
        endpoint("svc@corp", "my db", Some("tenant/one")).build_url("token"),
        "postgresql://svc%40corp:token@db.example:5432/my%20db?schema=tenant%2Fone"
    );
    assert_eq!(
        endpoint("svc%40corp", "my%20db", Some("tenant%2Fone")).build_url("token"),
        "postgresql://svc%40corp:token@db.example:5432/my%20db?schema=tenant%2Fone"
    );
    // exactly one decode happens, so a doubly encoded value stays encoded once more
    assert_eq!(
        endpoint("svc%2540corp", "name", None).build_url("token"),
        "postgresql://svc%2540corp:token@db.example:5432/name"
    );
    assert_eq!(
        endpoint("svc name", "name", None).build_url("token"),
        "postgresql://svc%20name:token@db.example:5432/name"
    );
}

#[test]
fn token_inserted_verbatim() {
    let token = "abc%2Fdef%3D%26x%2By%3Fz";
    assert_eq!(
        endpoint("user", "name", None).build_url(token),
        format!("postgresql://user:{token}@db.example:5432/name")
    );
}

#[test]
fn absent_or_empty_schema() {
    assert_eq!(
        endpoint("user", "name", None).build_url("token"),
        "postgresql://user:token@db.example:5432/name"
    );
    assert_eq!(
        endpoint("user", "name", Some("")).build_url("token"),
        "postgresql://user:token@db.example:5432/name"
    );
}

#[rstest]
#[case::no_username("postgresql://db.example/mydb")]
#[case::empty_username("postgresql://:pass@db.example/mydb")]
#[case::no_host("postgresql:///mydb")]
fn endpoint_missing_host_user(#[case] url: &str) {
    let error = parse_iam_endpoint_from_url(url).expect_err("the URL has no host or username");
    assert_eq!(
        error.to_string(),
        "Cannot parse IAM endpoint from URL: missing host or username"
    );
}

#[rstest]
#[case::no_path("postgresql://user@db.example")]
#[case::empty_path("postgresql://user@db.example/")]
fn endpoint_missing_name(#[case] url: &str) {
    let error = parse_iam_endpoint_from_url(url).expect_err("the URL has no database name");
    assert_eq!(
        error.to_string(),
        "Cannot parse IAM endpoint from URL: missing database name"
    );
}

#[test]
fn endpoint_first_schema_and_default_port() {
    let parsed = parse_iam_endpoint_from_url(
        "postgresql://sv%40corp@DB.Example/my%20db?schema=first&schema=second&schema=",
    )
    .expect("the URL parses");
    assert_eq!(
        parsed,
        IAMEndpoint {
            host: "db.example".to_owned(),
            port: "5432".to_owned(),
            user: "svc@corp".to_owned(),
            name: "my db".to_owned(),
            schema: Some("first".to_owned()),
        }
    );
    let zero = parse_iam_endpoint_from_url("postgresql://user@db.example:0/db?schema=")
        .expect("the zero port parses");
    assert_eq!(zero.port, "5432");
    assert_eq!(zero.schema, None);
    let explicit =
        parse_iam_endpoint_from_url("postgresql://user@db.example:6543/db").expect("parses");
    assert_eq!(explicit.port, "6543");
    let ipv6 = parse_iam_endpoint_from_url("postgresql://user@[::1]:5432/db").expect("parses");
    assert_eq!(ipv6.host, "::1");
}

#[rstest]
#[case::letters("postgresql://user@db.example:abc/db")]
#[case::over_range("postgresql://user@db.example:70000/db")]
#[case::negative("postgresql://user@db.example:-1/db")]
#[case::bracket("postgresql://[::1/db")]
fn endpoint_bad_port(#[case] url: &str) {
    parse_iam_endpoint_from_url(url).expect_err("the port or bracket is invalid");
}
