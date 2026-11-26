use super::systemone_entrypoint::{prepare_systemone_call, systemone_response};
use super::{DecisionsResponse, SystemOneArgs, SystemOneDeps, SystemOneEntryError};
use crate::api::decisions_send::clients::{DecisionsAsyncTransport, DecisionsSyncTransport};
use crate::api::decisions_send::send::DecisionsSender;

// litellm.asystemone: the same preparation and response check, over B3514's async send
pub async fn asystemone<Clients>(
    deps: SystemOneDeps<'_, Clients>,
    args: SystemOneArgs,
) -> Result<DecisionsResponse, SystemOneEntryError>
where
    Clients: DecisionsSyncTransport + DecisionsAsyncTransport,
{
    let call = prepare_systemone_call(deps.prepare, args)?;
    let mut sender = DecisionsSender::new(deps.clients, deps.env);
    let output = sender.asend(&call).await?;
    systemone_response(output)
}
