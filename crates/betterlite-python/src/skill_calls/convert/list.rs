use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{ListArguments, RequestError, common, field, provider};

pub(in crate::skill_calls) fn arguments(
    mut arguments: IndexMap<String, Json>,
) -> Result<ListArguments, RequestError> {
    let provider_name = provider(&mut arguments)?;
    let (params, litellm_call_id) = common(&provider_name, &mut arguments)?;
    let limit = match arguments.shift_remove("limit") {
        None | Some(Json::Null) => None,
        Some(Json::Number(number)) => Some(
            number
                .as_i64()
                .ok_or_else(|| RequestError::field(&provider_name, "limit"))?,
        ),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(RequestError::field(&provider_name, "limit"));
        }
    };
    let page = field::<String>(&provider_name, arguments.shift_remove("page"), "page")?;
    let source = field::<String>(&provider_name, arguments.shift_remove("source"), "source")?;
    Ok(ListArguments {
        params,
        limit,
        page,
        source,
        kwargs: arguments,
        litellm_call_id,
    })
}
