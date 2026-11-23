use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{RequestError, SkillArguments, common, field, provider};

pub(in crate::skill_calls) fn arguments(
    mut arguments: IndexMap<String, Json>,
) -> Result<SkillArguments, RequestError> {
    let provider_name = provider(&mut arguments)?;
    let (params, litellm_call_id) = common(&provider_name, &mut arguments)?;
    let skill_id = field::<String>(&provider_name, arguments.remove("skill_id"), "skill_id")?
        .ok_or_else(|| RequestError::field(&provider_name, "skill_id"))?;
    Ok(SkillArguments {
        params,
        skill_id,
        kwargs: arguments,
        litellm_call_id,
    })
}
