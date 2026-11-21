use std::collections::BTreeMap;
use std::convert::Infallible;

use async_trait::async_trait;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::management_rbac::{Raw, RawGeneral};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::management_admins::scope::AdminCaller;
use betterlite_server::management_rbac::guard::{AdminPrivileges, check_feature_access};
use betterlite_server::management_rbac::{ErrorDetail, FeatureAccessError, FeatureName};
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
struct Call {
    role: Option<UserRole>,
    feature: FeatureName,
    general_settings: RawGeneral,
}

#[derive(Debug, Serialize)]
struct Outcome {
    status: Option<u16>,
    detail: Option<ErrorDetail>,
}

// the Python side runs with prisma_client None, which B863 answers with False before any read
struct NoDatabase;

#[async_trait]
impl AdminPrivileges for NoDatabase {
    type Failure = Infallible;

    async fn has_admin_privileges(&self, _caller: &AdminCaller<'_>) -> Result<bool, Infallible> {
        Ok(false)
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut outcomes: Vec<Outcome> = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            outcomes.push(outcome(&runtime, call)?);
        }
        serde_json::to_value(outcomes).or_value_error()
    }
}

fn outcome(runtime: &Runtime, call: &Call) -> Result<Outcome, Box<HelperError>> {
    let raw = Raw {
        general_settings: call.general_settings.clone(),
    };
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let settings = raw.convert(&lookup).or_config_error()?;
    let checked = runtime.block_on(check_feature_access(
        call.role,
        None,
        call.feature,
        &settings,
        &NoDatabase,
    ));
    let Err(error) = checked else {
        return Ok(Outcome {
            status: None,
            detail: None,
        });
    };
    let status = error.status_code().as_u16();
    match error {
        FeatureAccessError::Disabled(feature) => Ok(Outcome {
            status: Some(status),
            detail: Some(feature.denial()),
        }),
        FeatureAccessError::Privileges(never) => match never {},
    }
}
