use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::agent_policy::{AgentResponse, ManagedAgentContext};
use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};

pub mod admit;
pub mod error;
pub mod invoke;
pub mod ports;
mod tests;

// The token an inbound managed request carries: LiteLLM's UserAPIKeyAuth constructor takes the context directly
pub fn managed_token(
    fields: BTreeMap<String, JsonData>,
    context: Option<ManagedAgentContext>,
) -> Result<UserAPIKeyAuth, AuthError> {
    let mut auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields))?;
    auth.extra.managed_agent_context = context;
    Ok(auth)
}

// The marks admission leaves on the auth that no public view exposes: the billing policy, the fresh-policy flag and the price
pub struct ManagedMarks {
    pub billing_agent_policy: Option<AgentResponse>,
    pub requires_fresh_policy: bool,
    pub agent_invocation_cost: Option<Decimal>,
}

#[must_use]
pub fn managed_marks(auth: &UserAPIKeyAuth) -> ManagedMarks {
    ManagedMarks {
        billing_agent_policy: auth.extra.billing_agent_policy.clone(),
        requires_fresh_policy: auth.extra.requires_fresh_policy,
        agent_invocation_cost: auth.extra.agent_invocation_cost,
    }
}
