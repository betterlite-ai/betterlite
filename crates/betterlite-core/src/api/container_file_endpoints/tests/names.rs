#![cfg(test)]

use betterlite_error::error::ErrorClass;

use super::super::{all_endpoint_names, async_endpoint_names};
use super::support::{TIMEOUT_SECONDS, fixture, list_call, logging};

const SYNC_AND_ASYNC_NAMES: [&str; 10] = [
    "list_container_files",
    "alist_container_files",
    "upload_container_file",
    "aupload_container_file",
    "retrieve_container_file",
    "aretrieve_container_file",
    "delete_container_file",
    "adelete_container_file",
    "retrieve_container_file_content",
    "aretrieve_container_file_content",
];

const ASYNC_NAMES: [&str; 5] = [
    "alist_container_files",
    "aupload_container_file",
    "aretrieve_container_file",
    "adelete_container_file",
    "aretrieve_container_file_content",
];

#[test]
fn names_all_returns_each_name_then_its_async_name() {
    let names = all_endpoint_names().expect("the catalog parses");
    assert_eq!(names, SYNC_AND_ASYNC_NAMES.map(str::to_owned).to_vec());
}

#[test]
fn names_async_returns_the_five_async_names_in_catalog_order() {
    let names = async_endpoint_names().expect("the catalog parses");
    assert_eq!(names, ASYNC_NAMES.map(str::to_owned).to_vec());
}

#[test]
fn names_unknown_endpoint_fails_before_any_collaborator_runs() {
    let fixture = fixture();
    let endpoints = fixture.endpoints();
    let failure = endpoints
        .container_file_call("list_containers", &list_call(), &logging(), TIMEOUT_SECONDS)
        .expect_err("the name is in neither catalog column");
    assert_eq!(
        failure.error.to_string(),
        "Unknown endpoint: list_containers"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.decoder.seen().is_empty());
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.seen().is_empty());
}
