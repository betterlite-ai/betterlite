use std::io::Write;

use serde::Serialize;

use super::{Client, DeleteUser, UsersError, post_user};

const DELETE_PATH: &str = "/user/delete";

#[derive(Serialize)]
struct DeleteUsersBody<'request> {
    user_ids: &'request [String],
}

pub(super) fn delete_user(
    options: &DeleteUser,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let body = serde_json::to_vec(&DeleteUsersBody {
        user_ids: &options.user_ids,
    })
    .map_err(UsersError::Encode)?;
    post_user(client, DELETE_PATH, body, stdout)
}
