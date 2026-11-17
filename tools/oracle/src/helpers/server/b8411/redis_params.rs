use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::proxy_request_contracts::coordination_redis::CoordinationRedisParams;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct RedisView<'a> {
    params: &'a CoordinationRedisParams,
    has_connection_target: bool,
}

pub(super) fn answer(body: &Value) -> Result<Value, Box<HelperError>> {
    let params = CoordinationRedisParams::from_value(body).or_value_error()?;
    let view = RedisView {
        has_connection_target: params.has_connection_target(),
        params: &params,
    };
    serde_json::to_value(view).or_value_error()
}
