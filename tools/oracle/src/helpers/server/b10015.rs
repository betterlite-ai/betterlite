use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::org_admin_keys::Settings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::org_admin_keys::{ErrorDetail, check_org_admin_key_generation};
use serde::Deserialize;
use serde::Serialize;

mod tests;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
struct Call {
    role: UserRole,
    // a case sends an org membership to prove the guard reads only the global role
    #[serde(default, rename = "org_id")]
    _organization: Option<String>,
    settings: Snapshot,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    #[serde(default, rename = "disable_key_generate_for_org_admin")]
    key_generation_disabled: bool,
}

#[derive(Debug, Serialize)]
struct Outcome {
    status: Option<u16>,
    detail: Option<ErrorDetail>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let outcomes: Vec<Outcome> = input.calls.iter().map(outcome).collect();
        serde_json::to_value(outcomes).map_err(|source| {
            Box::new(HelperError {
                class: "Output".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn outcome(call: &Call) -> Outcome {
    let settings = Settings {
        key_generation_disabled: call.settings.key_generation_disabled,
    };
    match check_org_admin_key_generation(call.role, &settings) {
        Ok(()) => Outcome {
            status: None,
            detail: None,
        },
        Err(error) => Outcome {
            status: Some(error.status_code().as_u16()),
            detail: Some(error.detail()),
        },
    }
}
