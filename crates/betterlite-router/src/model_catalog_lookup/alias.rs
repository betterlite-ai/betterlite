use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::model_catalog_lookup::error::LookupError;

// One model_group_alias value as LiteLLM stores it: the plain string form, the item mapping form, or anything listing skips.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AliasTarget {
    Name(String),
    Item {
        model: Option<Json>,
        hidden: Option<Json>,
    },
    Unsupported(Json),
}

// model_group_alias in configuration order, which the all-model listing keeps.
pub type AliasMap = IndexMap<String, AliasTarget>;

impl AliasTarget {
    // resolve_model_group_alias: the group a request is rewritten to, tolerant of an item without a usable model.
    #[must_use]
    pub fn resolved(&self) -> Option<&str> {
        match self {
            Self::Name(target) => (!target.is_empty()).then_some(target.as_str()),
            Self::Item {
                model: Some(Json::Text(model)),
                ..
            } => (!model.is_empty()).then_some(model.as_str()),
            Self::Item { .. } | Self::Unsupported(_) => None,
        }
    }

    // get_model_list_from_model_alias: the group an item lists. Both fields are read before hidden can skip it, as the item reads them.
    pub(crate) fn listed_model(&self, alias: &str, all: bool) -> Result<Option<&str>, LookupError> {
        match self {
            Self::Name(target) => Ok(Some(target.as_str())),
            Self::Item { model, hidden } => {
                let Some(flag) = hidden else {
                    return Err(LookupError::MalformedAlias {
                        alias: alias.to_owned(),
                        field: "hidden",
                    });
                };
                if all && matches!(flag, Json::Bool(true)) {
                    return Ok(None);
                }
                let Some(raw) = model else {
                    return Err(LookupError::MalformedAlias {
                        alias: alias.to_owned(),
                        field: "model",
                    });
                };
                let Json::Text(target) = raw else {
                    return Ok(None);
                };
                Ok(Some(target))
            }
            Self::Unsupported(_) => Ok(None),
        }
    }
}

// A configuration's aliases as this module reads them.
#[must_use]
pub fn from_config(aliases: &IndexMap<String, GroupAlias>) -> AliasMap {
    aliases
        .iter()
        .map(|(alias, entry)| {
            let target = AliasTarget::Item {
                model: Some(Json::Text(entry.model.clone())),
                hidden: Some(Json::Bool(entry.hidden)),
            };
            (alias.clone(), target)
        })
        .collect()
}
