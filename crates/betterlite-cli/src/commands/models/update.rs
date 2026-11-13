use std::io::Write;

use indexmap::IndexMap;
use serde::Serialize;

use crate::commands::Client;
use crate::commands::tables::write_json_response;

use super::params::parse_pairs;
use super::response::post_json;
use super::{ModelsError, UpdateModel};

const UPDATE_PATH: &str = "/model/update";

#[derive(Serialize)]
struct UpdateModelBody<'request> {
    id: &'request str,
    litellm_params: &'request IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_info: Option<&'request IndexMap<String, String>>,
}

pub(super) fn update_model(
    options: &UpdateModel,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    let params = parse_pairs(&options.param, "--param")?;
    let info = parse_pairs(&options.info, "--info")?;
    let model_info = (!options.info.is_empty()).then_some(&info);
    let body = serde_json::to_vec(&UpdateModelBody {
        id: &options.model_id,
        litellm_params: &params,
        model_info,
    })
    .map_err(ModelsError::Encode)?;
    let response = post_json(client, UPDATE_PATH, body)?;
    write_json_response(&response, stdout).map_err(|source| ModelsError::Response {
        path: UPDATE_PATH,
        source,
    })
}
