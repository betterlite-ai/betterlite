use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::secret_contracts::BaseSecretManager as _;
use crate::secret_contracts::options::{
    SecretOptions, SecretTag, SecretTags, SecretTimeout, SecretWrite,
};

use super::support::{
    DEFAULT_TIMEOUT, DEFAULT_URL, FakeTransport, SentTag, Settings, actions, answer, manager,
    sent_body, write_request,
};

fn tags_mapping() -> SecretTags {
    SecretTags::Mapping(BTreeMap::from([
        ("Environment".to_owned(), "Prod".to_owned()),
        ("Owner".to_owned(), "AI-Platform".to_owned()),
    ]))
}

#[tokio::test]
async fn create_secret_success() {
    let transport = FakeTransport::new(vec![answer(
        200,
        r#"{"ARN":"arn:one","Name":"prod/db","VersionId":"v1"}"#,
    )]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let response = (&manager)
        .async_write_secret(write_request("prod/db", &value, None, None))
        .await
        .unwrap();
    assert_eq!(
        response.body.expose_secret(),
        r#"{"ARN":"arn:one","Name":"prod/db","VersionId":"v1"}"#
    );
    let sent = transport.sent();
    assert_eq!(actions(&sent), ["secretsmanager.CreateSecret"]);
    assert_eq!(sent[0].url, DEFAULT_URL);
    assert_eq!(sent[0].timeout, DEFAULT_TIMEOUT);
    let body = sent_body(&sent[0].body);
    assert_eq!(body.name.as_deref(), Some("prod/db"));
    assert_eq!(body.secret_string.as_deref(), Some("s3cr3t"));
    assert_eq!(body.description, None);
    assert_eq!(body.kms_key_id, None);
    assert_eq!(body.tags, None);
    let token = body
        .client_request_token
        .expect("CreateSecret sends a token");
    Uuid::parse_str(&token).expect("the token is a version 4 uuid");
}

#[tokio::test]
async fn create_with_tags_dict() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:one"}"#)]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let tags = tags_mapping();
    (&manager)
        .async_write_secret(write_request("prod/db", &value, Some(&tags), None))
        .await
        .unwrap();
    let sent = transport.sent();
    let body = sent_body(&sent[0].body);
    assert_eq!(
        body.tags,
        Some(vec![
            SentTag {
                key: "Environment".to_owned(),
                value: "Prod".to_owned(),
            },
            SentTag {
                key: "Owner".to_owned(),
                value: "AI-Platform".to_owned(),
            },
        ])
    );
}

#[tokio::test]
async fn create_with_tags_list() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:one"}"#)]);
    let manager = manager(&transport, Settings::default());
    let value = SecretString::from("s3cr3t".to_owned());
    let tags = SecretTags::List(vec![
        SecretTag {
            key: "Owner".to_owned(),
            value: "AI-Platform".to_owned(),
        },
        SecretTag {
            key: "Environment".to_owned(),
            value: "Prod".to_owned(),
        },
    ]);
    (&manager)
        .async_write_secret(write_request("prod/db", &value, Some(&tags), None))
        .await
        .unwrap();
    let sent = transport.sent();
    let body = sent_body(&sent[0].body);
    assert_eq!(
        body.tags,
        Some(vec![
            SentTag {
                key: "Owner".to_owned(),
                value: "AI-Platform".to_owned(),
            },
            SentTag {
                key: "Environment".to_owned(),
                value: "Prod".to_owned(),
            },
        ]),
        "a list of {{Key, Value}} items passes through unchanged"
    );
}

#[tokio::test]
async fn create_with_description_and_kms() {
    let transport = FakeTransport::new(vec![answer(200, r#"{"ARN":"arn:one"}"#)]);
    let manager = manager(
        &transport,
        Settings {
            kms_key_id: Some("alias/gateway".to_owned()),
            replica_regions: Vec::new(),
        },
    );
    let value = SecretString::from("s3cr3t".to_owned());
    let request = SecretWrite {
        secret_name: "prod/db",
        secret_value: &value,
        description: Some("gateway key"),
        options: SecretOptions {
            optional_params: None,
            timeout: Some(SecretTimeout::Total(Duration::from_secs(3))),
        },
        tags: None,
    };
    (&manager).async_write_secret(request).await.unwrap();
    let sent = transport.sent();
    assert_eq!(sent[0].timeout, Duration::from_secs(3));
    let body = sent_body(&sent[0].body);
    assert_eq!(body.description.as_deref(), Some("gateway key"));
    assert_eq!(body.kms_key_id.as_deref(), Some("alias/gateway"));
}
