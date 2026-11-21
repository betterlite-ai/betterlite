#![cfg(test)]

use betterlite_openai::containers::container_file_http::transport::ContainerMethod;

use super::test_support::{
    CONTAINER_ID, CONTAINER_JSON, DELETED_JSON, FakeClient, Fixture, container_object,
    deleted_result, real_handler, real_ops,
};

#[test]
fn real_handler_retrieves_the_record_through_b3365() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some("https://api.example/v1".to_owned());
    let client = FakeClient::answering(CONTAINER_JSON);
    let handler = real_handler(client.clone());
    let ops = real_ops(&handler, &fixture);
    let request = fixture.request();

    let object = ops
        .retrieve_container(&request)
        .expect("the handler answers");

    assert_eq!(object, container_object());
    let requests = client.requests();
    let sent = requests.first().expect("one request");
    assert_eq!(sent.method, ContainerMethod::Get);
    assert_eq!(
        sent.url,
        format!("https://api.example/v1/containers/{CONTAINER_ID}")
    );
}

#[tokio::test]
async fn real_handler_retrieves_the_record_asynchronously() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some("https://api.example/v1".to_owned());
    let client = FakeClient::answering(CONTAINER_JSON);
    let handler = real_handler(client.clone());
    let ops = real_ops(&handler, &fixture);
    let request = fixture.request();

    let object = ops
        .aretrieve_container(&request)
        .await
        .expect("the handler answers");

    assert_eq!(object, container_object());
    assert_eq!(client.requests().len(), 1);
}

#[test]
fn real_handler_deletes_the_container_through_b3365() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some("https://api.example/v1".to_owned());
    let client = FakeClient::answering(DELETED_JSON);
    let handler = real_handler(client.clone());
    let ops = real_ops(&handler, &fixture);
    let request = fixture.request();

    let deleted = ops.delete_container(&request).expect("the handler answers");

    assert_eq!(deleted, deleted_result());
    assert_eq!(
        client.requests().first().expect("one request").method,
        ContainerMethod::Delete
    );
}

#[tokio::test]
async fn real_handler_deletes_the_container_asynchronously() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some("https://api.example/v1".to_owned());
    let client = FakeClient::answering(DELETED_JSON);
    let handler = real_handler(client.clone());
    let ops = real_ops(&handler, &fixture);
    let request = fixture.request();

    let deleted = ops
        .adelete_container(&request)
        .await
        .expect("the handler answers");

    assert_eq!(deleted, deleted_result());
    assert_eq!(client.requests().len(), 1);
}
