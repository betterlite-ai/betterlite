use std::io::Write;

use http::Method;

use super::{Client, GetUser, Payload, Request, UsersError, send_and_write};

const INFO_PATH: &str = "/user/info";

pub(super) fn get_user(
    options: &GetUser,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), UsersError> {
    let mut url = client.resolve(INFO_PATH)?;
    if let Some(user_id) = &options.user_id {
        url.query_pairs_mut().append_pair("user_id", user_id);
    }
    let request = Request {
        method: Method::GET,
        url,
        headers: vec![(
            String::from("content-type"),
            String::from("application/json"),
        )],
        payload: Payload::Empty,
    };
    send_and_write(client, request, INFO_PATH, stdout)
}
