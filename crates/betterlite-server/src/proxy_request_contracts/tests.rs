#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use super::ContractError;
use super::budget::{BudgetDeleteRequest, BudgetNewRequest, BudgetRequest};
use super::coordination_redis::{CoordinationRedisNode, CoordinationRedisParams};
use super::cost_estimate::{CostEstimateRequest, CostEstimateResponse};
use super::customer::NewCustomerRequest;
use super::key::{KeyRequest, ResetSpendRequest};
use super::user::info::{UserInfoResponse, UserInfoV2Response};
use super::user::{
    ChangePasswordRequest, ChangePasswordResponse, NewUserRequest, NewUserRequestTeam,
    UpdateUserFields, UpdateUserRequest,
};

pub(crate) fn contract(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("a JSON fixture")
}

pub(crate) fn validation_text(error: &ContractError) -> String {
    match error {
        ContractError::Validation { message } => message.clone(),
        ContractError::Decode { source } => panic!("a decode error: {source}"),
    }
}

pub(crate) fn assert_validation(error: &ContractError, expected: &str) {
    let message = validation_text(error);
    assert_eq!(message, expected);
}

pub(crate) fn decode_text(error: &ContractError) -> String {
    match error {
        ContractError::Decode { source } => source.to_string(),
        ContractError::Validation { message } => panic!("a validation error: {message}"),
    }
}

pub(crate) fn assert_decode(error: ContractError) {
    match error {
        ContractError::Decode { .. } => {}
        ContractError::Validation { message } => panic!("a validation error: {message}"),
    }
}

#[test]
fn rejects_a_body_that_is_not_an_object() {
    let array = KeyRequest::from_value(&contract("[]"));
    assert_decode(array.expect_err("an array is not a request body"));
    let number = NewCustomerRequest::from_value(&contract("5"));
    assert_decode(number.expect_err("a number is not a request body"));
    let null = CoordinationRedisParams::from_value(&contract("null"));
    assert_decode(null.expect_err("a null is not a request body"));
}

// Every record decodes the same corpus input, so a fuzzer-found input runs through the whole contract set.
fn check_every_record(value: &JsonValue) {
    drop(KeyRequest::from_value(value));
    drop(ResetSpendRequest::from_value(value));
    drop(NewUserRequestTeam::from_value(value));
    drop(NewUserRequest::from_value(value));
    drop(UpdateUserFields::from_value(value));
    drop(UpdateUserRequest::from_value(value));
    drop(ChangePasswordRequest::from_value(value));
    drop(ChangePasswordResponse::from_value(value));
    drop(UserInfoResponse::from_value(value));
    drop(UserInfoV2Response::from_value(value));
    drop(BudgetNewRequest::from_value(value));
    drop(BudgetRequest::from_value(value));
    drop(BudgetDeleteRequest::from_value(value));
    drop(NewCustomerRequest::from_value(value));
    drop(CostEstimateRequest::from_value(value));
    drop(CostEstimateResponse::from_value(value));
    drop(CoordinationRedisNode::from_value(value));
    drop(CoordinationRedisParams::from_value(value));
}

#[rstest]
fn replays_the_proxy_request_contracts_corpus(
    #[files("fuzz/corpus/proxy_request_contracts/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a corpus seed");
    if let Ok(value) = serde_json::from_slice::<JsonValue>(&bytes) {
        check_every_record(&value);
    }
}
