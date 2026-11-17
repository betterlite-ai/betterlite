use indexmap::IndexMap;

use crate::auth_model_lists::access_groups::AccessGroupScope;
use crate::auth_model_lists::lists::append_unique;
use crate::auth_model_lists::wildcard::{ExpansionContext, WildcardError, wildcard_models};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListingFlags {
    pub infer_model_from_keys: bool,
    pub return_wildcard_routes: bool,
    pub groups: AccessGroupScope,
}
#[derive(Clone, Copy, Debug)]
pub struct ListingRequest<'request> {
    pub key_models: &'request [String],
    pub team_models: &'request [String],
    pub user_model: Option<&'request str>,
    pub team_id: Option<&'request str>,
    pub model_access_groups: &'request IndexMap<String, Vec<String>>,
    pub flags: ListingFlags,
}
pub fn get_complete_model_list(
    proxy_model_list: &[String],
    request: &ListingRequest<'_>,
    context: &ExpansionContext<'_>,
) -> Result<Vec<String>, WildcardError> {
    let mut unique_models: Vec<String> = Vec::new();
    if !request.key_models.is_empty() {
        append_unique(&mut unique_models, request.key_models.iter().cloned());
    } else if !request.team_models.is_empty() {
        append_unique(&mut unique_models, request.team_models.iter().cloned());
    } else {
        append_unique(&mut unique_models, proxy_model_list.iter().cloned());
        if request.flags.groups.include {
            append_unique(
                &mut unique_models,
                request.model_access_groups.keys().cloned(),
            );
        }
        if let Some(user_model) = request.user_model.filter(|model| !model.is_empty()) {
            append_unique(&mut unique_models, [user_model.to_owned()]);
        }
        if request.flags.infer_model_from_keys
            && let Some(valid_models) = context.providers.valid_models(None, None)
        {
            append_unique(&mut unique_models, valid_models);
        }
    }
    if request.flags.groups.only {
        let access_groups_only = unique_models
            .into_iter()
            .filter(|model| request.model_access_groups.contains_key(model))
            .collect();
        return Ok(access_groups_only);
    }
    let all_wildcard_models = wildcard_models(
        &mut unique_models,
        request.flags.return_wildcard_routes,
        request.team_id,
        context,
    )?;
    unique_models.extend(all_wildcard_models);
    Ok(unique_models)
}
