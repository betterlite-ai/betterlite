use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::resource_repository::table::KeyFilter;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use serde::Deserialize;

use super::error::RouterWeightError;
use super::model_info::{RouterWeightModelInfo, RouterWeightModelInfoError};

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(try_from = "DeploymentFixture")]
pub struct RouterWeightDeployment {
    pub model_name: String,
    pub model_info: RouterWeightModelInfo,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentFixture {
    model_name: String,
    model_info: Json,
}

impl TryFrom<DeploymentFixture> for RouterWeightDeployment {
    type Error = RouterWeightModelInfoError;

    fn try_from(fixture: DeploymentFixture) -> Result<Self, Self::Error> {
        Ok(Self {
            model_name: fixture.model_name,
            model_info: RouterWeightModelInfo::decode(&fixture.model_info)?,
        })
    }
}

impl RouterWeightDeployment {
    pub(crate) fn from_stored(row: &ProxyModelTable) -> Result<Self, RouterWeightModelInfoError> {
        let model_info = match &row.fields.model_info {
            Some(payload) => {
                let text =
                    serde_json::to_string(&payload.0).map_err(RouterWeightModelInfoError::Json)?;
                let value: Json =
                    serde_json::from_str(&text).map_err(RouterWeightModelInfoError::Json)?;
                RouterWeightModelInfo::decode(&value)?
            }
            None => RouterWeightModelInfo::default(),
        };
        Ok(Self {
            model_name: row.fields.model_name.clone(),
            model_info,
        })
    }

    pub(crate) fn from_configured(
        spec: &DeploymentSpec,
    ) -> Result<Self, RouterWeightModelInfoError> {
        Ok(Self {
            model_name: spec.model_name.clone(),
            model_info: RouterWeightModelInfo::decode(&Json::Object(spec.model_info.clone()))?,
        })
    }
}

pub fn validate_weight_reference(
    model_group: &str,
    deployment_id: &str,
    team_id: Option<&str>,
    stored: Option<&RouterWeightDeployment>,
    configured: Option<&RouterWeightDeployment>,
) -> Result<(), RouterWeightError> {
    let Some(reference) = stored.or(configured) else {
        return Err(RouterWeightError::UnknownDeployment {
            deployment_id: deployment_id.to_owned(),
        });
    };
    let hidden = (stored.is_none() && reference.model_info.db_model)
        || reference
            .model_info
            .team_id
            .as_deref()
            .is_some_and(|owner| Some(owner) != team_id);
    if hidden {
        return Err(RouterWeightError::UnknownDeployment {
            deployment_id: deployment_id.to_owned(),
        });
    }
    let public_name = reference.model_info.team_public_model_name.as_str();
    let canonical_group = if reference.model_info.team_id.is_some() && !public_name.is_empty() {
        public_name
    } else {
        reference.model_name.as_str()
    };
    if model_group != canonical_group {
        return Err(RouterWeightError::WrongModelGroup {
            deployment_id: deployment_id.to_owned(),
            model_group: model_group.to_owned(),
        });
    }
    Ok(())
}

pub(crate) async fn stored_deployments(
    repository: &ModelRepository<'_>,
    deployment_ids: &BTreeSet<&str>,
) -> Result<BTreeMap<String, RouterWeightDeployment>, RouterWeightError> {
    let table = repository.table().map_err(|error| {
        if matches!(error, DbError::NoDbConnected) {
            return RouterWeightError::DatabaseUnavailable;
        }
        RouterWeightError::Database(TableError::from(error))
    })?;
    let rows = table
        .as_ref()
        .find::<JsonData>(KeyFilter::All)
        .await
        .map_err(|error| {
            if matches!(error, TableError::Database(DbError::NoDbConnected)) {
                return RouterWeightError::DatabaseUnavailable;
            }
            RouterWeightError::Database(error)
        })?;
    let mut stored = BTreeMap::new();
    for row in rows {
        let JsonData::Object(fields) = &row else {
            continue;
        };
        let Some(JsonData::String(model_id)) = fields.get("model_id") else {
            continue;
        };
        if !deployment_ids.contains(model_id.as_str()) {
            continue;
        }
        let stored_id = model_id.clone();
        let decoded = ProxyModelTable::try_from(row)?;
        stored.insert(stored_id, RouterWeightDeployment::from_stored(&decoded)?);
    }
    Ok(stored)
}
