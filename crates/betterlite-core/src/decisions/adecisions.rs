use super::decisions_entrypoint::prepare_decisions_call;
use super::{DecisionsArgs, DecisionsDeps, DecisionsEntryError};
use crate::api::decisions_send::clients::{DecisionsAsyncTransport, DecisionsSyncTransport};
use crate::api::decisions_send::format::DecisionsOutput;
use crate::api::decisions_send::send::DecisionsSender;

// litellm.adecisions: _prepare_decisions_call, then B3514's async send
pub async fn adecisions<Clients>(
    deps: DecisionsDeps<'_, Clients>,
    args: DecisionsArgs,
) -> Result<DecisionsOutput, DecisionsEntryError>
where
    Clients: DecisionsSyncTransport + DecisionsAsyncTransport,
{
    let call = prepare_decisions_call(deps.prepare, args)?;
    let mut sender = DecisionsSender::new(deps.clients, deps.env);
    sender.asend(&call).await.map_err(DecisionsEntryError::from)
}
