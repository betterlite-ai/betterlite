use std::collections::BTreeSet;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_server::proxy_request_contracts::user::UpdateUserRequest;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct UpdateUserView<'request> {
    user_id: Option<&'request String>,
    user_email: Option<&'request String>,
    supplied: &'request BTreeSet<String>,
}

pub(super) fn answer(body: &Value) -> Result<Value, Box<HelperError>> {
    let request = UpdateUserRequest::from_value(body).or_value_error()?;
    let view = UpdateUserView {
        user_id: request.user_id.as_ref(),
        user_email: request.user_email.as_ref(),
        supplied: &request.fields.supplied,
    };
    serde_json::to_value(view).or_value_error()
}
