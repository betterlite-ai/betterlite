use std::sync::Arc;

use betterlite_core::decisions::adecisions;
use betterlite_core::systemone::asystemone;
use betterlite_http::interactions::base::clients::HttpClients;

use super::super::convert::{CallWire, ConvertError, decisions_answer, response_answer};
use super::super::error::SessionFailure;
use super::decisions::{DecisionFormat, decision_deps};
use super::{CODE_INTERPRETER, CREATE, DECISIONS, DELETE, RUN, SYSTEMONE, SessionCore};

impl SessionCore {
    pub(crate) async fn acall(&self, wire: &CallWire) -> Result<String, SessionFailure> {
        match wire.operation.as_str() {
            DECISIONS => self.adecision_call(wire, DecisionFormat::Decisions).await,
            SYSTEMONE => self.adecision_call(wire, DecisionFormat::SystemOne).await,
            CREATE => self.create_sandbox(wire).await,
            RUN => self.run_code(wire).await,
            DELETE => self.delete_sandbox(wire).await,
            CODE_INTERPRETER => self.code_interpreter(wire).await,
            other => Err(SessionFailure::from(ConvertError::Operation {
                operation: other.to_owned(),
            })),
        }
    }

    async fn adecision_call(
        &self,
        wire: &CallWire,
        format: DecisionFormat,
    ) -> Result<String, SessionFailure> {
        let resolution = self.resolution(&wire.settings.global_credentials());
        let env = self.pre_post_env();
        let mut clients = HttpClients::new(self.tls.clone(), Arc::clone(&self.proxy));
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
}
