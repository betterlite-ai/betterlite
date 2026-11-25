use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::error::LookupError;
use betterlite_router::model_group_lookups::access_groups::AccessGroupMap;

use crate::health_access_and_connections::visibility::{HealthRouter, LookupFailure};

// A router an oracle case answers from: each row is a model group and deployment id, and fail makes the lookup error
#[cfg_attr(test, mutants::skip)]
#[must_use]
pub fn fake_health_router<R>(rows: R, fail: bool) -> Box<dyn HealthRouter>
where
    R: IntoIterator<Item = (String, String)>,
{
    Box::new(FakeRouter {
        deployments: rows.into_iter().collect(),
        fail,
    })
}

struct FakeRouter {
    deployments: Vec<(String, String)>,
    fail: bool,
}

impl HealthRouter for FakeRouter {
    #[cfg_attr(test, mutants::skip)]
    fn get_deployment(&self, model_id: &str) -> Result<Option<DeploymentSpec>, LookupFailure> {
        if self.fail {
            return Err(LookupFailure::Lookup(LookupError::InvalidRow { index: 0 }));
        }
        Ok(self
            .deployments
            .iter()
            .find(|(_, id)| id == model_id)
            .map(|(model_name, id)| {
                let mut model_info = JsonObject::new();
                model_info.insert("id".to_owned(), Json::Text(id.clone()));
                DeploymentSpec {
                    model_name: model_name.clone(),
                    litellm_params: JsonObject::new(),
                    model_info,
                    extra: JsonObject::new(),
                }
            }))
    }
    #[cfg_attr(test, mutants::skip)]
    fn get_model_list(
        &self,
        _model_name: Option<&str>,
        _team_id: Option<&str>,
    ) -> Result<Vec<DeploymentSpec>, LookupFailure> {
        Ok(Vec::new())
    }
    #[cfg_attr(test, mutants::skip)]
    fn get_model_names(&self, _team_id: Option<&str>) -> Result<Vec<String>, LookupFailure> {
        Ok(Vec::new())
    }
    #[cfg_attr(test, mutants::skip)]
    fn get_model_access_groups(&self) -> Result<AccessGroupMap, LookupFailure> {
        Ok(AccessGroupMap::new())
    }
}
