use std::io::Write;

use indexmap::IndexMap;
use serde::Serialize;

use crate::commands::Client;

use super::params::parse_pairs;
use super::response::{post_json, write_json};
use super::{AddModel, ModelsError};

const NEW_PATH: &str = "/model/new";

#[derive(Serialize)]
struct NewModelBody<'request> {
    model_name: &'request str,
    litellm_params: &'request IndexMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_info: Option<&'request IndexMap<String, String>>,
}

pub(super) fn add_model(
    options: &AddModel,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelsError> {
    let params = parse_pairs(&options.param, "--param")?;
    let info = parse_pairs(&options.info, "--info")?;
    let model_info = (!options.info.is_empty()).then_some(&info);
    let body = serde_json::to_vec(&NewModelBody {
        model_name: &options.model_name,
        litellm_params: &params,
        model_info,
    })
    .map_err(ModelsError::Encode)?;
    let response = post_json(client, NEW_PATH, body)?;
    write_json(NEW_PATH, &response, stdout)
}
