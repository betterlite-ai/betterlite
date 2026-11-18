use crate::budget_permission_rows::Timestamp;
use crate::key_repository::table::TokenValue;

use super::UserUpdate;
use super::columns::{UserColumn, UserColumnOp, UserUpdateOps, UserWrite};

pub(super) fn update_operations(
    request: &UserUpdate,
    create: &UserWrite,
    now: Timestamp,
) -> UserUpdateOps {
    let mut update = match (&request.update_fields, &request.custom_update_fields) {
        (Some(fields), _) => UserUpdateOps::from(fields),
        (None, Some(ops)) => ops.clone(),
        (None, None) => UserUpdateOps::from(create),
    };
    if !update.0.is_empty() {
        update
            .0
            .entry(UserColumn::UpdatedAt)
            .or_insert(UserColumnOp::Set(TokenValue::Timestamp(now)));
    }
    update
}
