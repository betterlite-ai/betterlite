use std::future::Future;
use std::num::NonZeroU64;
use std::sync::Arc;

use betterlite_auth::auth_identity::roles::effective_user_role;
use betterlite_auth::session_token_codec::builders::{CliSession, SessionTokenCodec};
use betterlite_db::identity_rows::OptionalPayload;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_mcp::gateway_dcr::ports::{
    ConsentTeam, LookupConsentTeams, MintProxyCredential, MintedProxyCredential,
    ProxyCredentialMintFailure, ReloadUserFailure,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, StoredCipher};
use rust_decimal::Decimal;

use crate::cli_team_grants::selection::selected_cli_sso_team_detail;

use super::{ActiveSubjects, SharedNonces, SubjectStore, UserRowSource};

mod tests;

const SECONDS_PER_HOUR: i64 = 3_600;

const fn mint_failure(failure: ReloadUserFailure) -> ProxyCredentialMintFailure {
    match failure {
        ReloadUserFailure::Unresolvable => ProxyCredentialMintFailure::Unresolvable,
        ReloadUserFailure::Unavailable => ProxyCredentialMintFailure::Unavailable,
        ReloadUserFailure::Faulted => ProxyCredentialMintFailure::Faulted,
        ReloadUserFailure::NoActiveKey => ProxyCredentialMintFailure::NoActiveKey,
    }
}

impl<S: SubjectStore> ActiveSubjects<S> {
    async fn consent_team_list(
        &self,
        user_id: &str,
    ) -> Result<Vec<ConsentTeam>, ReloadUserFailure> {
        let user = match self.load_active_user(user_id, UserRowSource::Cache).await {
            Ok(user) => user,
            Err(failure) => return Err(failure),
        };
        let Some(details) = self.store.team_details(&user.teams).await else {
            return Err(ReloadUserFailure::Unavailable);
        };
        Ok(details
            .into_iter()
            .filter_map(|detail| {
                let team_id = detail.team_id?;
                Some(ConsentTeam {
                    team_id,
                    team_alias: detail.team_alias,
                })
            })
            .collect())
    }

    async fn minted_credential(
        &self,
        user_id: &str,
        team_id: Option<&str>,
    ) -> Result<MintedProxyCredential, ProxyCredentialMintFailure> {
        let user = match self
            .load_active_user(user_id, UserRowSource::Database)
            .await
        {
            Ok(user) => user,
            Err(failure) => return Err(mint_failure(failure)),
        };
        if let Some(wanted) = team_id
            && !user.teams.iter().any(|team| team.as_str() == wanted)
        {
            return Err(ProxyCredentialMintFailure::NotAMember);
        }
        let details = if user.teams.is_empty() {
            Vec::new()
        } else {
            let Some(details) = self.store.team_details(&user.teams).await else {
                return Err(ProxyCredentialMintFailure::Unavailable);
            };
            details
        };
        if team_id.is_none() && details.iter().any(|detail| detail.team_id.is_some()) {
            return Err(ProxyCredentialMintFailure::TeamRequired);
        }
        let Some(selected) = selected_cli_sso_team_detail(&details, team_id) else {
            return Err(ProxyCredentialMintFailure::NotAMember);
        };
        let row = LiteLLMUserTable {
            user_id: user.user_id.clone(),
            user_alias: None,
            team_id: None,
            sso_user_id: None,
            organization_id: None,
            object_permission_id: None,
            password: None,
            password_reset_required: None,
            last_breach_check_at: None,
            teams: Vec::new(),
            user_role: Some(effective_user_role(user.user_role.as_deref()).to_string()),
            max_budget: None,
            spend: Decimal::ZERO,
            user_email: None,
            models: user.models.clone(),
            metadata: None,
            max_parallel_requests: None,
            tpm_limit: None,
            rpm_limit: None,
            budget_duration: None,
            budget_reset_at: None,
            allowed_cache_controls: Vec::new(),
            policies: Vec::new(),
            model_spend: OptionalPayload::default(),
            model_max_budget: OptionalPayload::default(),
            created_at: None,
            updated_at: None,
            organization_memberships: None,
            object_permission: None,
        };
        let session = CliSession {
            team_id: team_id.map(str::to_owned),
            team_alias: selected.team_alias,
            team_models: selected.team_models,
            team_model_aliases: selected.team_model_aliases,
            max_budget: None,
        };
        let codec = SessionTokenCodec {
            cipher: StoredCipher {
                settings: CipherSettings {
                    signing_key: self.secret.clone(),
                    algorithm: Algorithm::Aes256Gcm,
                },
                nonces: SharedNonces(Arc::clone(&self.nonces)),
                diagnostics: TracingDiagnostics,
            },
            max_ui_session_budget: None,
            cli_expiration_hours: self.settings.cli_token_hours,
        };
        let key = match codec.cli_session_token(&row, &session, self.clock.now()) {
            Ok(key) => key,
            Err(error) => {
                tracing::debug!(error = %error, "the CLI session token could not be built");
                return Err(ProxyCredentialMintFailure::Unresolvable);
            }
        };
        let lifetime = self
            .settings
            .cli_token_hours
            .saturating_mul(SECONDS_PER_HOUR);
        let Some(expires_in) = u64::try_from(lifetime).ok().and_then(NonZeroU64::new) else {
            return Err(ProxyCredentialMintFailure::Unresolvable);
        };
        Ok(MintedProxyCredential {
            key,
            expires_in,
            user_id: user.user_id,
            team_id: team_id.map(str::to_owned),
        })
    }
}

impl<S: SubjectStore> LookupConsentTeams for ActiveSubjects<S> {
    fn consent_teams(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Vec<ConsentTeam>, ReloadUserFailure>> + Send {
        self.consent_team_list(user_id)
    }
}

impl<S: SubjectStore> MintProxyCredential for ActiveSubjects<S> {
    fn mint(
        &self,
        user_id: &str,
        team_id: Option<&str>,
    ) -> impl Future<Output = Result<MintedProxyCredential, ProxyCredentialMintFailure>> + Send
    {
        self.minted_credential(user_id, team_id)
    }
}
