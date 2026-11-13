use std::io::Write;

use serde::Serialize;

use crate::commands::Client;

use super::response::{post_json, write_json};
use super::{DeleteModel, ModelsError};

const DELETE_PATH: &str = "/model/delete";

#[derive(Serialize)]
struct DeleteBody<'request> {
    id: &'request str,
}

pub(super) fn delete_model(
    options: &DeleteModel,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    let body = serde_json::to_vec(&DeleteBody {
        id: &options.model_id,
    })
    .map_err(ModelsError::Encode)?;
    let response = post_json(client, DELETE_PATH, body)?;
    write_json(DELETE_PATH, &response, stdout)
}
