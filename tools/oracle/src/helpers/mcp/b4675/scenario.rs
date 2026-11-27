use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_authorize::authorize::{
    AggregateAuthorize, AuthorizeRequest, NativeClientAuthorize, aggregate_authorize,
    native_client_authorize,
};
use betterlite_mcp::gateway_authorize::connect::{
    CompleteFlowRequest, ConnectFlowCall, ConnectPorts, FlowForm, complete_connect_flow,
    describe_connect_flow,
};
use betterlite_mcp::gateway_authorize::contract::native_client_auth_contract;
use betterlite_mcp::gateway_authorize::reply::{GatewayReply, RequestView};
use betterlite_mcp::gateway_dcr::single_use::SingleUseGuard;
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use super::answers::{Answers, ScriptedTokens};
use super::input::{Input, ReplyKind, Step};
use super::report::{contract_report, step_report};
use super::support::{
    BASE, NoBackend, answers, cipher, client_ids, failure, guard, registry, view,
};

pub(super) struct Scenario {
    cipher: DbCipher,
    clients: Vec<String>,
    registry: ServerRegistry,
    runtime: Runtime,
    guard: SingleUseGuard<NoBackend>,
}

struct Prepared {
    view: RequestView,
    answers: Answers,
    tokens: ScriptedTokens,
    client_id: String,
}

impl Scenario {
    pub(super) fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let cipher = cipher();
        let clients = client_ids(&input.clients, &cipher);
        let registry = registry(&input.servers)?;
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure(&error))?;
        Ok(Self {
            cipher,
            clients,
            registry,
            runtime,
            guard: guard(),
        })
    }

    pub(super) fn report(
        &self,
        step: &Step,
        captured: &[Option<String>],
    ) -> (Value, Option<String>) {
        let Some(kind) = step.kind.reply_kind() else {
            let contract = native_client_auth_contract(BASE, step.token_exchange);
            return (contract_report(&step.name, &contract), None);
        };
        let prepared = Prepared {
            view: view(step, sent(captured, step.cookie_from)),
            answers: answers(step),
            tokens: ScriptedTokens::new(&step.tokens),
            client_id: self.client_id(step),
        };
        step_report(&step.name, self.reply(kind, step, &prepared))
    }

    fn client_id(&self, step: &Step) -> String {
        if step.client < 0 {
            return step.client_id.clone();
        }
        let index = usize::try_from(step.client).unwrap_or_default();
        self.clients.get(index).cloned().unwrap_or_default()
    }

    fn reply(&self, kind: ReplyKind, step: &Step, prepared: &Prepared) -> GatewayReply {
        match kind {
            ReplyKind::Aggregate => self.aggregate(step, prepared),
            ReplyKind::Native => self.native(step, prepared),
            ReplyKind::Describe => self.describe(step, prepared),
            ReplyKind::Complete => self.complete(step, prepared),
        }
    }

    fn aggregate(&self, step: &Step, prepared: &Prepared) -> GatewayReply {
        let request = authorize_request(&prepared.client_id, step);
        aggregate_authorize(
            &AggregateAuthorize {
                view: &prepared.view,
                request: &request,
                session_user_id: step.user.as_deref(),
                resource: step.resource.as_deref(),
                now: step.now,
            },
            &self.registry,
            &self.cipher,
            &prepared.tokens,
        )
    }

    fn native(&self, step: &Step, prepared: &Prepared) -> GatewayReply {
        let request = authorize_request(&prepared.client_id, step);
        let call = NativeClientAuthorize {
            view: &prepared.view,
            request: &request,
            session_user_id: step.user.as_deref(),
            now: step.now,
        };
        self.runtime.block_on(native_client_authorize(
            &call,
            &prepared.answers,
            &prepared.answers,
            &self.cipher,
            &prepared.tokens,
        ))
    }

    fn describe(&self, step: &Step, prepared: &Prepared) -> GatewayReply {
        let ports = self.ports(prepared);
        let call = ConnectFlowCall {
            view: &prepared.view,
            handle: &step.handle,
            session_user_id: step.user.as_deref(),
            now: step.now,
        };
        self.runtime
            .block_on(describe_connect_flow(&call, &ports, &self.cipher))
    }

    fn complete(&self, step: &Step, prepared: &Prepared) -> GatewayReply {
        let ports = self.ports(prepared);
        let call = ConnectFlowCall {
            view: &prepared.view,
            handle: &step.handle,
            session_user_id: step.user.as_deref(),
            now: step.now,
        };
        let form = FlowForm {
            delivery: step.delivery.as_deref(),
            team_id: step.team_id.as_deref(),
            decision: step.decision.as_deref(),
        };
        let request = CompleteFlowRequest { call, form };
        self.runtime.block_on(complete_connect_flow(
            &request,
            &ports,
            &self.cipher,
            &self.guard,
            &prepared.tokens,
        ))
    }

    fn ports<'scenario>(
        &'scenario self,
        prepared: &'scenario Prepared,
    ) -> ConnectPorts<'scenario, Answers, Answers> {
        ConnectPorts {
            registry: &self.registry,
            vendor: &prepared.answers,
            reachability: &prepared.answers,
        }
    }
}

fn authorize_request<'request>(
    client_id: &'request str,
    step: &'request Step,
) -> AuthorizeRequest<'request> {
    AuthorizeRequest {
        client_id,
        redirect_uri: &step.redirect_uri,
        state: &step.state,
        code_challenge: step.code_challenge.as_deref(),
        code_challenge_method: step.code_challenge_method.as_deref(),
        response_type: step.response_type.as_deref(),
    }
}

fn sent(captured: &[Option<String>], source: i64) -> Option<String> {
    if source < 0 {
        return None;
    }
    let index = usize::try_from(source).unwrap_or_default();
    captured.get(index).cloned().flatten()
}
