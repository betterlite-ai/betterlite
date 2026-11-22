use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, LiteLLMObjectPermissionTable};
use betterlite_db::team_rows::aliases::ModelAliases;
use betterlite_db::team_rows::member::Member;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;
use serde::Serialize;

pub type ModelAliasMap = BTreeMap<String, String>;

#[derive(Debug, thiserror::Error)]
pub enum TeamColumnsError {
    #[error("model alias {alias} does not name a model")]
    AliasNotText { alias: String },
}

// _TeamJsonColumns: the two loose team columns, re-read with the shape the badge needs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamJsonColumns {
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub model_aliases: Option<ModelAliasMap>,
}

// The team_* fields of UserAPIKeyAuth; a caller with no team applies none and keeps the defaults.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TeamGrants {
    pub team_alias: Option<String>,
    pub team_tpm_limit: Option<i64>,
    pub team_rpm_limit: Option<i64>,
    pub team_tpd_limit: Option<i64>,
    #[serde(serialize_with = "rust_decimal::serde::float_option::serialize")]
    pub team_max_budget: Option<Decimal>,
    #[serde(serialize_with = "rust_decimal::serde::float_option::serialize")]
    pub team_soft_budget: Option<Decimal>,
    pub team_model_max_budget: Option<BTreeMap<String, JsonData>>,
    #[serde(serialize_with = "rust_decimal::serde::float_option::serialize")]
    pub team_spend: Option<Decimal>,
    pub team_models: Vec<String>,
    pub team_blocked: bool,
    pub team_metadata: Option<BTreeMap<String, JsonData>>,
    pub team_model_aliases: Option<ModelAliasMap>,
    pub team_object_permission_id: Option<String>,
    pub team_object_permission: Option<LiteLLMObjectPermissionTable>,
    pub team_member: Option<Member>,
    #[serde(serialize_with = "rust_decimal::serde::float_option::serialize")]
    pub team_member_spend: Option<Decimal>,
    pub team_member_tpm_limit: Option<i64>,
    pub team_member_rpm_limit: Option<i64>,
}

// TeamModelAliasTable.model_aliases: stored text is parsed, a stored mapping is checked as it is
pub fn decode_model_aliases(
    column: &ModelAliases,
) -> Result<Option<ModelAliasMap>, TeamColumnsError> {
    match column {
        // writers hand the column json.dumps text; text that is not a text-to-text object reads as no aliases
        ModelAliases::Text(text) => Ok(serde_json::from_str(text).ok()),
        ModelAliases::Map(aliases) => {
            let mut models = ModelAliasMap::new();
            for (alias, model) in aliases {
                let JsonData::String(name) = model else {
                    return Err(TeamColumnsError::AliasNotText {
                        alias: alias.clone(),
                    });
                };
                models.insert(alias.clone(), name.clone());
            }
            Ok(Some(models))
        }
    }
}

// team_model_aliases(team) is this projection's model_aliases; no global alias is merged in
#[must_use]
pub fn team_json_columns(team: &LiteLLMTeamTable) -> TeamJsonColumns {
    let column = team
        .fields
        .litellm_model_table
        .as_ref()
        .and_then(|table| table.model_aliases.as_ref());
    // both columns validate together, so an alias mapping that fails discards the metadata too
    column.map(decode_model_aliases).transpose().map_or(
        TeamJsonColumns {
            metadata: None,
            model_aliases: None,
        },
        |model_aliases| TeamJsonColumns {
            metadata: team.fields.base.metadata.clone(),
            model_aliases: model_aliases.flatten(),
        },
    )
}

// The membership is projected as given: its ids are not checked against the team or the user.
#[must_use]
pub fn team_grants(
    team: &LiteLLMTeamTable,
    membership: Option<&LiteLLMTeamMembership>,
    user_id: Option<&str>,
) -> TeamGrants {
    let fields = &team.fields;
    let columns = team_json_columns(team);
    // the first entry whose user_id equals a supplied id, an empty one included; emails never match
    let team_member = fields
        .base
        .members_with_roles
        .iter()
        .find(|member| user_id.is_some() && member.base.user_id.as_deref() == user_id)
        .cloned();
    TeamGrants {
        team_alias: fields.base.team_alias.clone(),
        team_tpm_limit: fields.base.tpm_limit,
        team_rpm_limit: fields.base.rpm_limit,
        team_tpd_limit: fields.base.tpd_limit,
        team_max_budget: fields.base.max_budget,
        team_soft_budget: fields.base.soft_budget,
        team_model_max_budget: fields.model_max_budget.0.clone(),
        team_spend: fields.spend,
        team_models: fields.base.models.clone(),
        team_blocked: fields.base.blocked,
        team_metadata: columns.metadata,
        team_model_aliases: columns.model_aliases,
        team_object_permission_id: fields.object_permission_id.clone(),
        team_object_permission: fields.object_permission.clone(),
        team_member,
        team_member_spend: membership.and_then(|row| row.spend.0),
        team_member_tpm_limit: membership
            .and_then(LiteLLMTeamMembership::safe_get_team_member_tpm_limit),
        team_member_rpm_limit: membership
            .and_then(LiteLLMTeamMembership::safe_get_team_member_rpm_limit),
    }
}
