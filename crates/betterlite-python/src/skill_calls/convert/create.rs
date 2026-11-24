use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{CreateArguments, RequestError, common, field, provider};

pub(in crate::skill_calls) fn arguments(
    mut arguments: IndexMap<String, Json>,
) -> Result<CreateArguments, RequestError> {
    let provider_name = provider(&mut arguments)?;
    let (params, litellm_call_id) = common(&provider_name, &mut arguments)?;
    let display_title = field::<String>(
        &provider_name,
        arguments.shift_remove("display_title"),
        "display_title",
    )?;
    let files = match arguments.shift_remove("files") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(items),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(RequestError::field(&provider_name, "files"));
        }
    };
    let extra_body = field::<IndexMap<String, Json>>(
        &provider_name,
        arguments.shift_remove("extra_body"),
        "extra_body",
    )?;
    Ok(CreateArguments {
        params,
        display_title,
        files,
        extra_body,
        kwargs: arguments,
        litellm_call_id,
    })
}
