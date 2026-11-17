#![no_main]

use betterlite_config::sections::settings::JsonValue;
use betterlite_server::proxy_request_contracts::budget::{
    BudgetDeleteRequest, BudgetNewRequest, BudgetRequest,
};
use betterlite_server::proxy_request_contracts::coordination_redis::{
    CoordinationRedisNode, CoordinationRedisParams,
};
use betterlite_server::proxy_request_contracts::cost_estimate::{
    CostEstimateRequest, CostEstimateResponse,
};
use betterlite_server::proxy_request_contracts::customer::NewCustomerRequest;
use betterlite_server::proxy_request_contracts::key::{KeyRequest, ResetSpendRequest};
use betterlite_server::proxy_request_contracts::user::info::{
    UserInfoResponse, UserInfoV2Response,
};
use betterlite_server::proxy_request_contracts::user::{
    ChangePasswordRequest, ChangePasswordResponse, NewUserRequest, NewUserRequestTeam,
    UpdateUserFields, UpdateUserRequest,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(value) = serde_json::from_slice::<JsonValue>(bytes) else {
        return;
    };
    drop(KeyRequest::from_value(&value));
    drop(ResetSpendRequest::from_value(&value));
    drop(NewUserRequestTeam::from_value(&value));
    drop(NewUserRequest::from_value(&value));
    drop(UpdateUserFields::from_value(&value));
    drop(UpdateUserRequest::from_value(&value));
    drop(ChangePasswordRequest::from_value(&value));
    drop(ChangePasswordResponse::from_value(&value));
    drop(UserInfoResponse::from_value(&value));
    drop(UserInfoV2Response::from_value(&value));
    drop(BudgetNewRequest::from_value(&value));
    drop(BudgetRequest::from_value(&value));
    drop(BudgetDeleteRequest::from_value(&value));
    drop(NewCustomerRequest::from_value(&value));
    drop(CostEstimateRequest::from_value(&value));
    drop(CostEstimateResponse::from_value(&value));
    drop(CoordinationRedisNode::from_value(&value));
    drop(CoordinationRedisParams::from_value(&value));
});
