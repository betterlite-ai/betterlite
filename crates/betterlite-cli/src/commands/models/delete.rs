use std::io::Write;

use serde::Serialize;

use crate::commands::Client;
use crate::commands::tables::write_json_response;

use super::response::post_json;
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
    write_json_response(&response, stdout).map_err(|source| ModelsError::Response {
        path: DELETE_PATH,
        source,
    })
}
