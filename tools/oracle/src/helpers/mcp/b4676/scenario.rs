use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::errors::SessionKeyError;
use betterlite_mcp::gateway_dcr::contracts::{GATEWAY_AUTH_CODE_PREFIX, REFRESH_CLAIM_KEY_PREFIX};
use betterlite_mcp::gateway_dcr::models::GatewayAuthCode;
use betterlite_mcp::gateway_dcr::sealed::seal;
use betterlite_mcp::gateway_dcr::single_use::{ClaimOutcome, SingleUseGuard};
use betterlite_mcp::gateway_token::grants::{TokenForm, TokenPorts, aggregate_token};
use betterlite_mcp::gateway_token::introspection::introspect_gateway_token;
use betterlite_mcp::gateway_token::revocation::{RevokeRequest, revoke_refresh_token};
use betterlite_mcp::gateway_token::support::reply::TokenReply;
use betterlite_mcp::gateway_token::support::{SigningSetup, pkce_verifier_matches};
use betterlite_mcp::registry::servers::ServerRegistry;
use betterlite_mcp::session_credentials::SessionSigningConfigError;
use betterlite_mcp::session_tokens::{
    SESSION_REFRESH_TTL_SECONDS, mint_session_token, open_session_token,
};
use betterlite_mcp::types::{SessionPrincipal, SessionSigningKeys, SessionTokenKind};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use super::input::{Input, MasterMode, SetupKind, SetupStep, Step, StepKind};
use super::normalize::{matches_report, token_report};
use super::support::{
    Answers, CaseDefect, FIXED_JTI, FixedEntropy, NoBackend, cipher, client_ids, guard,
    present_setup, registry,
};

pub(super) struct Scenario {
    cipher: DbCipher,
    clients: Vec<String>,
    registry: ServerRegistry,
    runtime: Runtime,
    present: SigningSetup,
    absent: SigningSetup,
    invalid: SigningSetup,
    guard: SingleUseGuard<NoBackend>,
    products: Vec<String>,
    base_url: String,
    steps: Vec<Step>,
}

impl Scenario {
    pub(super) fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let cipher = cipher();
        let clients = client_ids(&input.clients, &cipher)?;
        let registry = registry(&input.servers)?;
        let runtime = Builder::new_current_thread().build().or_config_error()?;
        let mut scenario = Self {
            cipher,
            clients,
            registry,
            runtime,
            present: present_setup()?,
            absent: SigningSetup::NoMasterKey,
            invalid: SigningSetup::Invalid(SessionSigningConfigError::Keys(
                SessionKeyError::EmptyKid,
            )),
            guard: guard(),
            products: Vec::new(),
            base_url: input.base_url.clone(),
            steps: input.steps.clone(),
        };
        let products = scenario.setup_products(&input.setup)?;
        scenario.products = products;
        Ok(scenario)
    }

    pub(super) fn reports(&self) -> Result<Vec<Value>, Box<HelperError>> {
        let mut reports = Vec::with_capacity(self.steps.len());
        for step in &self.steps {
            reports.push(self.report(step)?);
        }
        Ok(reports)
    }

    fn report(&self, step: &Step) -> Result<Value, Box<HelperError>> {
        let now = timestamp(step.now)?;
        let reply = match step.kind {
            StepKind::Token => self.token(step, now)?,
            StepKind::Revoke => {
                let token = entry(&self.products, step.token, &step.token_text);
                let client = entry(&self.clients, step.client, &step.client_id);
                let request = RevokeRequest {
                    token: &token,
                    client_id: Some(&client),
                };
                let setup = self.setup(step.master);
                self.runtime.block_on(revoke_refresh_token(
                    &request,
                    setup,
                    &self.guard,
                    &self.cipher,
                    now,
                ))
            }
            StepKind::Introspect => self.introspect(step, now)?,
            StepKind::Pkce => {
                let matched = pkce_verifier_matches(&step.code_verifier, &step.code_challenge);
                return Ok(matches_report(step.name.as_str(), matched));
            }
        };
        Ok(token_report(
            step.name.as_str(),
            &reply,
            self.session_keys(),
            now,
            &self.clients,
        ))
    }

    fn introspect(&self, step: &Step, now: DateTime<Utc>) -> Result<TokenReply, Box<HelperError>> {
        let token = entry(&self.products, step.token, &step.token_text);
        let answers = Answers::new(step)?;
        let setup = self.setup(step.master);
        Ok(self.runtime.block_on(introspect_gateway_token(
            &token,
            setup,
            &answers,
            &self.guard,
            now,
        )))
    }

    fn token(&self, step: &Step, now: DateTime<Utc>) -> Result<TokenReply, Box<HelperError>> {
        let client = entry(&self.clients, step.client, &step.client_id);
        let code = entry(&self.products, step.code, &step.code_text);
        let refresh = entry(&self.products, step.refresh, &step.refresh_text);
        let subject = entry(&self.products, step.subject, &step.subject_text);
        let answers = Answers::new(step)?;
        let entropy = FixedEntropy;
        let form = TokenForm {
            grant_type: Some(&step.grant_type),
            code: Some(&code),
            redirect_uri: step.redirect_uri.as_deref(),
            client_id: Some(&client),
            code_verifier: Some(&step.code_verifier),
            refresh_token: Some(&refresh),
            resource: step.resource.as_deref(),
            subject_token: Some(&subject),
            subject_token_type: step.subject_token_type.as_deref(),
            requested_token_type: step.requested_token_type.as_deref(),
        };
        let ports = TokenPorts {
            registry: &self.registry,
            cipher: &self.cipher,
            base_url: &self.base_url,
            entropy: &entropy,
            reload_user: &answers,
            mint_proxy_credential: &answers,
            guard: &self.guard,
            exchange_subject_token: &answers,
            exchange_context: &(),
        };
        let setup = self.setup(step.master);
        Ok(self
            .runtime
            .block_on(aggregate_token(setup, &form, &ports, now)))
    }

    const fn setup(&self, mode: MasterMode) -> &SigningSetup {
        match mode {
            MasterMode::Present => &self.present,
            MasterMode::Absent => &self.absent,
            MasterMode::Invalid => &self.invalid,
        }
    }

    const fn session_keys(&self) -> Option<&SessionSigningKeys> {
        match &self.present {
            SigningSetup::Ready(keys) => Some(keys),
            SigningSetup::NoMasterKey | SigningSetup::Invalid(_) => None,
        }
    }

    fn setup_products(&self, steps: &[SetupStep]) -> Result<Vec<String>, Box<HelperError>> {
        let mut products = Vec::with_capacity(steps.len());
        for step in steps {
            let product = match step.kind {
                SetupKind::MintCode => self.mint_code(step),
                SetupKind::MintToken => self.mint_token(step),
                SetupKind::BurnRefresh => self.burn_refresh(step, &products),
            }?;
            products.push(product);
        }
        Ok(products)
    }

    fn mint_code(&self, step: &SetupStep) -> Result<String, Box<HelperError>> {
        let sealed = seal(
            GATEWAY_AUTH_CODE_PREFIX,
            &auth_code(step, &self.clients),
            &self.cipher,
        );
        sealed.or_value_error()
    }

    fn mint_token(&self, step: &SetupStep) -> Result<String, Box<HelperError>> {
        let keys = self
            .session_keys()
            .ok_or_else(|| CaseDefect("minting a setup token needs the master key".to_owned()))
            .or_value_error()?;
        let principal = SessionPrincipal {
            user_id: step.user_id.clone(),
            client_id: entry(&self.clients, step.client, &step.client_id),
            resource_server_id: step.resource_server_id.clone(),
            audience: step.audience,
            team_id: step.team_id.clone(),
        };
        let at = timestamp(step.at)?;
        let minted = mint_session_token(&principal, step.token_kind, keys, at, || Ok(FIXED_JTI))
            .or_value_error()?;
        let mut token = minted.token.expose_secret().to_owned();
        if step.tamper {
            token.push('x');
        }
        if step.bearer {
            token = format!("Bearer {token}");
        }
        Ok(token)
    }

    fn burn_refresh(
        &self,
        step: &SetupStep,
        products: &[String],
    ) -> Result<String, Box<HelperError>> {
        let keys = self
            .session_keys()
            .ok_or_else(|| CaseDefect("burning a setup token needs the master key".to_owned()))
            .or_value_error()?;
        let at = timestamp(step.at)?;
        let token = entry(products, step.token, &step.token_text);
        let opened =
            open_session_token(&token, SessionTokenKind::Refresh, keys, at).or_value_error()?;
        let claim_key = format!("{REFRESH_CLAIM_KEY_PREFIX}{}", opened.jti);
        let outcome = self
            .runtime
            .block_on(self.guard.claim(&claim_key, SESSION_REFRESH_TTL_SECONDS));
        match outcome {
            ClaimOutcome::First | ClaimOutcome::Replayed | ClaimOutcome::Unavailable => {
                Ok(String::new())
            }
        }
    }
}

fn auth_code(step: &SetupStep, clients: &[String]) -> GatewayAuthCode {
    GatewayAuthCode {
        user_id: step.user_id.clone(),
        client_id: entry(clients, step.client, &step.client_id),
        redirect_uri: step.redirect_uri.clone(),
        code_challenge: step.code_challenge.clone(),
        jti: step.jti.clone(),
        iat: step.iat,
        exp: step.exp,
        resource_server_id: step.resource_server_id.clone(),
        audience: step.audience,
        team_id: step.team_id.clone(),
    }
}

fn entry(entries: &[String], index: i64, literal: &str) -> String {
    if index < 0 {
        return literal.to_owned();
    }
    usize::try_from(index)
        .ok()
        .and_then(|position| entries.get(position))
        .cloned()
        .unwrap_or_default()
}

fn timestamp(seconds: i64) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::from_timestamp(seconds, 0)
        .ok_or_else(|| CaseDefect(format!("the case timestamp {seconds} is out of range")))
        .or_value_error()
}
