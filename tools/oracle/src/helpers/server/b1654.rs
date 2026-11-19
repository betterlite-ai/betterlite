use betterlite_config::sections::settings::JsonValue;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::compliance_checks::checker::ComplianceChecker;
use betterlite_server::compliance_checks::decode::decode_compliance_request;
use betterlite_server::compliance_checks::model::ComplianceRequest;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    regulation: Regulation,
    request: JsonValue,
}

#[derive(Deserialize)]
enum Regulation {
    #[serde(rename = "eu-ai-act")]
    EuAiAct,
    #[serde(rename = "gdpr")]
    Gdpr,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let bytes = serde_json::to_vec(&input.request)?;
        let request: ComplianceRequest = decode_compliance_request(&bytes).or_value_error()?;
        let checker = ComplianceChecker::new(&request);
        let response = match input.regulation {
            Regulation::EuAiAct => checker.eu_ai_act_response(),
            Regulation::Gdpr => checker.gdpr_response(),
        };
        let value = serde_json::to_value(response)?;
        Ok(value)
    }
}
