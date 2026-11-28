use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::jwt_claims::claims::JwtClaims;
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_team_selection::claims_teams::{find_team_in_claims, get_all_team_ids};
use betterlite_auth::jwt_team_selection::header::{HeaderTeam, resolve_team_from_header};
use betterlite_auth::jwt_team_selection::object::{ObjectPolicy, validate_object_id};
use betterlite_auth::jwt_team_selection::settings::SelectionSettings;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_paths::paths::Document;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// What an upstream JWT verification stage hands this stage: the verified claims, the team lookup and the request facts selection reads.
pub struct JwtTeamSelectionRule {
    pub claims: Document,
    pub claims_settings: JwtAuthSettings,
    pub lookup: Arc<TeamLookup>,
    pub request_headers: BTreeMap<String, String>,
    pub team_id_upsert: Option<bool>,
    pub enforce_rbac: bool,
    pub user_id: Option<String>,
    pub is_proxy_admin: bool,
}

impl fmt::Debug for JwtTeamSelectionRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JwtTeamSelectionRule")
            .field("claims", &self.claims)
            .field("request_headers", &self.request_headers)
            .field("team_id_upsert", &self.team_id_upsert)
            .field("enforce_rbac", &self.enforce_rbac)
            .field("user_id", &self.user_id)
            .field("is_proxy_admin", &self.is_proxy_admin)
            .finish_non_exhaustive()
    }
}

// The team the claims and the x-litellm-team-id header selected; B877 builds the request context from these.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub claim_team_id: Option<String>,
    pub header: Option<HeaderTeam>,
}

#[derive(Clone, Debug)]
pub struct JwtTeamSelectionStageInput {
    pub context: AuthContext,
    pub rule: Option<Arc<JwtTeamSelectionRule>>,
    pub settings: Option<SelectionSettings>,
    pub selection: Selection,
}

// the auth loop builds one StageInput per backend; JWT requests carry their rule, every other request passes through
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let settings = request
        .extensions()
        .get::<Arc<ServerState>>()
        .map(|state| state.jwt_team_selection.settings.clone());
    Ok(StageInput::JwtTeamSelection(Box::new(
        JwtTeamSelectionStageInput {
            context,
            rule: request
                .extensions()
                .get::<Arc<JwtTeamSelectionRule>>()
                .cloned(),
            settings,
            selection: Selection::default(),
        },
    )))
}

// the team reads behind claim and header resolution, awaited before dispatch because it holds no request borrow
pub(crate) async fn prefetch(
    stage: &mut JwtTeamSelectionStageInput,
) -> Result<(), Box<ProxyException>> {
    let (Some(rule), Some(settings)) = (stage.rule.clone(), stage.settings.as_ref()) else {
        return Ok(());
    };
    let claims = JwtClaims {
        claims: rule.claims.clone(),
        settings: &rule.claims_settings,
        trusted_issuers: &[],
    };
    let (claim_team_id, _team) =
        find_team_in_claims(&claims, &rule.lookup, settings, rule.team_id_upsert)
            .await
            .map_err(|error| Box::new(error.into_proxy_exception()))?;
    let mut allowed =
        get_all_team_ids(&claims).map_err(|error| Box::new(error.into_proxy_exception()))?;
    if let Some(team_id) = &claim_team_id {
        allowed.insert(team_id.clone());
    }
    let header = resolve_team_from_header(
        Some(&rule.request_headers),
        &allowed,
        settings.routing.fallback_to_db_teams,
        &rule.lookup,
    )
    .await
    .map_err(|error| Box::new(error.into_proxy_exception()))?;
    stage.selection = Selection {
        claim_team_id,
        header,
    };
    Ok(())
}

pub(crate) async fn prefetched(
    mut stage: Box<JwtTeamSelectionStageInput>,
) -> Result<StageInput, StageFailure> {
    prefetch(&mut stage)
        .await
        .map_err(StageFailure::JwtTeamSelection)?;
    Ok(StageInput::JwtTeamSelection(stage))
}

// validate_object_id needs the team the earlier phases selected, so it runs once prefetch has stored it
pub(crate) fn run_stage(stage: &JwtTeamSelectionStageInput) -> Result<AuthContext, StageFailure> {
    let Some(rule) = stage.rule.as_deref() else {
        return Ok(stage.context.clone());
    };
    let team_id = stage.selection.claim_team_id.as_deref().or_else(|| {
        stage
            .selection
            .header
            .as_ref()
            .map(|header| header.team_id.as_str())
    });
    validate_object_id(
        rule.user_id.as_deref(),
        team_id,
        ObjectPolicy {
            enforce_rbac: rule.enforce_rbac,
            is_proxy_admin: rule.is_proxy_admin,
        },
    )
    .map_err(|error| StageFailure::JwtTeamSelection(Box::new(error.into_proxy_exception())))?;
    Ok(stage.context.clone())
}
