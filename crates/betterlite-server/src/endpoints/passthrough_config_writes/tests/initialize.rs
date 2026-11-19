use betterlite_auth::pass_through_registry::PassThroughRouteType;
use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::endpoints::passthrough_config_writes::initialize::InitializeError;
use crate::endpoints::passthrough_config_writes::initialize::initialize_pass_through_endpoints_in_db;

use super::support;

#[tokio::test]
async fn initialize_installs_the_stored_endpoints() {
    let saved = [
        support::stored_with("ep1", "/one", r#", "include_subpath": true"#),
        support::stored_with("ep2", "/two", r#", "auth": false"#),
    ];
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::entries(&saved)),
    };
    let routes = support::registry();
    let config = support::FakeConfig::of(&[]);
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    let registrations = support::registered(&routes);
    assert_eq!(registrations.len(), 3);
    assert_eq!(
        support::registered_paths(&routes),
        vec!["/one".to_owned(), "/one".to_owned(), "/two".to_owned()]
    );
    let subpath = registrations.get(1).expect("the subpath route");
    assert_eq!(subpath.route_type, PassThroughRouteType::Subpath);
    assert_eq!(subpath.auth, Some(true));
    let open = registrations.get(2).expect("the public route");
    assert_eq!(open.auth, Some(false));
    let protected = support::openai_routes(&routes);
    assert!(protected.contains(&"/one".to_owned()));
    assert!(!protected.contains(&"/two".to_owned()));
}

#[tokio::test]
async fn initialize_registers_the_config_endpoints_next_to_the_stored_ones() {
    let saved = [support::stored("ep1", "/one")];
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::entries(&saved)),
    };
    let routes = support::registry();
    let configured =
        [r#"{"id": "cfg", "path": "/conf", "target": "https://conf.example.com"}"#.to_owned()];
    let config = support::FakeConfig::of(&configured);
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    assert_eq!(
        support::registered_ids(&routes),
        vec!["ep1".to_owned(), "cfg".to_owned()]
    );
    assert_eq!(
        support::registered_paths(&routes),
        vec!["/one".to_owned(), "/conf".to_owned()]
    );
}

#[rstest]
#[case::missing(false)]
#[case::read_fails(true)]
#[tokio::test]
async fn initialize_serves_the_config_endpoints_without_a_stored_field(#[case] read_fails: bool) {
    let reply = if read_fails {
        support::ReaderReply::Fails
    } else {
        support::ReaderReply::Missing
    };
    let reader = support::FakeReader { reply };
    let routes = support::registry();
    let configured = [r#"{"path": "/conf", "target": "https://conf.example.com"}"#.to_owned()];
    let config = support::FakeConfig::of(&configured);
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    assert_eq!(support::registered_paths(&routes), vec!["/conf".to_owned()]);
}

#[tokio::test]
async fn initialize_treats_a_stored_value_that_is_not_a_list_as_no_endpoints() {
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::json(r#"{"path": "/one"}"#)),
    };
    let routes = support::registry();
    let config = support::FakeConfig { entries: None };
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    assert!(support::registered(&routes).is_empty());
}

#[tokio::test]
async fn initialize_registers_a_stored_entry_without_auth_authenticated() {
    let saved = [support::stored("ep1", "/one")];
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::entries(&saved)),
    };
    let routes = support::registry();
    let config = support::FakeConfig { entries: None };
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    let registrations = support::registered(&routes);
    assert_eq!(registrations.first().expect("the route").auth, Some(true));
    assert!(support::openai_routes(&routes).contains(&"/one".to_owned()));
}

#[tokio::test]
async fn initialize_registers_a_config_entry_without_auth_unauthenticated() {
    let reader = support::FakeReader {
        reply: support::ReaderReply::Missing,
    };
    let routes = support::registry();
    let configured = [r#"{"path": "/conf", "target": "https://conf.example.com"}"#.to_owned()];
    let config = support::FakeConfig::of(&configured);
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    let registrations = support::registered(&routes);
    assert_eq!(registrations.first().expect("the route").auth, Some(false));
    assert!(!support::openai_routes(&routes).contains(&"/conf".to_owned()));
}

#[tokio::test]
async fn initialize_gives_a_stored_entry_without_an_id_a_generated_one() {
    let saved = [r#"{"path": "/one", "target": "https://target.example.com"}"#.to_owned()];
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::entries(&saved)),
    };
    let routes = support::registry();
    let config = support::FakeConfig { entries: None };
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect("initialize");
    let registrations = support::registered(&routes);
    let registration = registrations.first().expect("the route");
    uuid::Uuid::parse_str(&registration.endpoint_id).unwrap();
}

#[tokio::test]
async fn initialize_resolves_a_stored_header_from_the_environment() {
    let saved = [support::stored_with(
        "ep1",
        "/one",
        r#", "headers": {"X-Key": "os.environ/UPSTREAM"}"#,
    )];
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::entries(&saved)),
    };
    let routes = support::registry();
    let config = support::FakeConfig { entries: None };
    initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[("UPSTREAM", "secret")]),
    )
    .await
    .expect("initialize");
    let params = support::params_of(&routes, "ep1");
    let headers = params.custom_headers.expect("custom headers");
    assert_eq!(headers.get("X-Key"), Some(&Json::Text("secret".to_owned())));
}

#[tokio::test]
async fn initialize_answers_an_error_for_a_stored_entry_it_cannot_decode() {
    let saved = [r#"{"id": "ep1", "path": "/one"}"#.to_owned()];
    let reader = support::FakeReader {
        reply: support::ReaderReply::Field(support::entries(&saved)),
    };
    let routes = support::registry();
    let config = support::FakeConfig { entries: None };
    let error = initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect_err("a decode error");
    assert!(matches!(error, InitializeError::Record(_)));
    assert!(support::registered(&routes).is_empty());
}

#[tokio::test]
async fn initialize_answers_an_error_for_a_config_entry_it_cannot_decode() {
    let reader = support::FakeReader {
        reply: support::ReaderReply::Missing,
    };
    let routes = support::registry();
    let configured = [r#"{"path": 5}"#.to_owned()];
    let config = support::FakeConfig::of(&configured);
    let error = initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect_err("a decode error");
    assert!(matches!(error, InitializeError::Config(_)));
}

#[tokio::test]
async fn initialize_answers_an_error_for_a_config_entry_without_a_path() {
    let reader = support::FakeReader {
        reply: support::ReaderReply::Missing,
    };
    let routes = support::registry();
    let configured = [r#"{"target": "https://conf.example.com"}"#.to_owned()];
    let config = support::FakeConfig::of(&configured);
    let error = initialize_pass_through_endpoints_in_db(
        &reader,
        &routes,
        &config,
        &support::secrets(),
        &support::environment(&[]),
    )
    .await
    .expect_err("a missing path");
    assert!(matches!(error, InitializeError::Registration(_)));
}
