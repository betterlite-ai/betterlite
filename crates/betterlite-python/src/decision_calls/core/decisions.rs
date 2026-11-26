use std::sync::Arc;

use betterlite_core::decisions::{DecisionsDeps, DecisionsPrepare, decisions};
use betterlite_core::systemone::systemone;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_models::resolution::ResolutionContext;

use super::super::convert::{CallWire, decisions_answer, response_answer};
use super::super::error::SessionFailure;
use super::{SessionCore, azure_ai};

#[derive(Clone, Copy)]
pub(super) enum DecisionFormat {
    Decisions,
    SystemOne,
}

pub(super) fn sync_decision(
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
            let output = decisions(deps, wire.arguments.decisions()?)?;
            decisions_answer(&output).map_err(SessionFailure::from)
        }
        DecisionFormat::SystemOne => {
            let output = systemone(deps, wire.arguments.systemone()?)?;
            response_answer(&output, &output.hidden_params).map_err(SessionFailure::from)
        }
    }
}

pub(super) fn decision_deps<'deps, Clients>(
    wire: &'deps CallWire,
    resolution: &'deps ResolutionContext<'deps>,
    env: &'deps PrePostEnv<'deps>,
    clients: &'deps mut Clients,
) -> DecisionsDeps<'deps, Clients> {
    DecisionsDeps {
        clients,
        env,
        prepare: DecisionsPrepare {
            resolution,
            globals: wire.settings.llm_globals(),
            azure_ai: azure_ai(),
            logging: None,
            global_drop_params: wire.settings.global_drop_params(),
        },
    }
}
