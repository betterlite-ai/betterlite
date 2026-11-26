use std::sync::Arc;

use betterlite_core::decisions::adecisions;
use betterlite_core::systemone::asystemone;
use betterlite_http::interactions::base::clients::HttpClients;

use super::super::convert::{CallWire, decisions_answer, response_answer};
use super::super::error::SessionFailure;
use super::SessionCore;
use super::decisions::{DecisionFormat, decision_deps};

pub(super) async fn adecision_call(
    core: &SessionCore,
    wire: &CallWire,
    format: DecisionFormat,
) -> Result<String, SessionFailure> {
    let resolution = core.resolution(&wire.settings.global_credentials());
    let env = core.pre_post_env();
    let mut clients = HttpClients::new(core.tls.clone(), Arc::clone(&core.proxy));
    let deps = decision_deps(wire, &resolution, &env, &mut clients);
    match format {
        DecisionFormat::Decisions => {
            let output = adecisions(deps, wire.arguments.decisions()?).await?;
            decisions_answer(&output).map_err(SessionFailure::from)
        }
        DecisionFormat::SystemOne => {
            let output = asystemone(deps, wire.arguments.systemone()?).await?;
            response_answer(&output, &output.hidden_params).map_err(SessionFailure::from)
        }
    }
}
