#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;

use crate::sqs_logging::credentials::{CredentialParams, EnvironmentCredentials};
use crate::sqs_logging::tests::support::{
    FixedClock, RecordingDelivery, environment, fixed_time, payload,
};

use super::{DynamoClient as _, HttpDynamoClient, PutItemError};
use crate::legacy_dynamodb::projection;

const ENDPOINT: &str = "https://dynamodb.us-east-1.amazonaws.com/";

fn client(
    delivery: RecordingDelivery,
    variables: &[(&str, &str)],
) -> HttpDynamoClient<RecordingDelivery> {
    HttpDynamoClient {
        delivery,
        credentials: Arc::new(EnvironmentCredentials {
            environment: Arc::new(environment(variables)),
        }),
        params: CredentialParams::default(),
        clock: Arc::new(FixedClock(fixed_time())),
        endpoint: ENDPOINT.to_owned(),
        region: "us-east-1".to_owned(),
    }
}

fn credentials() -> &'static [(&'static str, &'static str)] {
    &[
        ("AWS_ACCESS_KEY_ID", "access"),
        ("AWS_SECRET_ACCESS_KEY", "secret"),
    ]
}

fn entry<'item>(item: &'item Json, name: &str) -> &'item Json {
    let Json::Object(fields) = item else {
        panic!("the body is an object");
    };
    fields.get(name).expect("the field is present")
}

#[tokio::test]
async fn posts_a_signed_put_item() {
    let delivery = RecordingDelivery::delivering();
    let records = Arc::clone(&delivery.records);
    let client = client(delivery, credentials());
    let record = projection::item(&payload()).expect("the item builds");
    client
        .put_item("logs", &record)
        .await
        .expect("the put succeeds");
    let recorded = records.snapshot();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, ENDPOINT);
    assert_eq!(
        recorded[0].headers["content-type"],
        "application/x-amz-json-1.0"
    );
    assert_eq!(
        recorded[0].headers["x-amz-target"],
        "DynamoDB_20120810.PutItem"
    );
    assert!(recorded[0].headers.contains_key("authorization"));
    let body = serde_json::from_slice::<Json>(&recorded[0].body).expect("the body is JSON");
    assert_eq!(entry(&body, "TableName"), &Json::Text("logs".to_owned()));
    let item = entry(&body, "Item");
    let id = entry(item, "id");
    let value = entry(id, "S");
    assert_eq!(value, &Json::Text("sqs-health-check".to_owned()));
}

#[tokio::test]
async fn reports_a_rejected_status() {
    let client = client(RecordingDelivery::answering(400, ""), credentials());
    let record = projection::item(&payload()).expect("the item builds");
    let error = client
        .put_item("logs", &record)
        .await
        .expect_err("the answer is an error");
    assert!(matches!(error, PutItemError::Status { status: 400 }));
}

#[tokio::test]
async fn reports_a_transport_failure() {
    let client = client(RecordingDelivery::failing(), credentials());
    let record = projection::item(&payload()).expect("the item builds");
    let error = client
        .put_item("logs", &record)
        .await
        .expect_err("the delivery fails");
    assert!(matches!(error, PutItemError::Delivery(_)));
}

#[tokio::test]
async fn reports_missing_credentials() {
    let client = client(RecordingDelivery::delivering(), &[]);
    let record = projection::item(&payload()).expect("the item builds");
    let error = client
        .put_item("logs", &record)
        .await
        .expect_err("no credentials are configured");
    assert!(matches!(error, PutItemError::Credentials(_)));
}

#[tokio::test]
async fn reports_an_unparsable_endpoint() {
    let mut client = client(RecordingDelivery::delivering(), credentials());
    client.endpoint = "not a url".to_owned();
    let record = projection::item(&payload()).expect("the item builds");
    let error = client
        .put_item("logs", &record)
        .await
        .expect_err("the endpoint is invalid");
    assert!(matches!(error, PutItemError::Url(_)));
}
