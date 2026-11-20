use betterlite_auth::sso_mappings::headers::decode_generic_headers;
use betterlite_auth::sso_mappings::load::{load_role_mappings, load_team_mappings};
use betterlite_auth::sso_mappings::repository::SsoSettingsRows;
use betterlite_auth::sso_mappings::settings::{GenericSsoSettings, generic_sso_settings};
use betterlite_auth::sso_types::records::{RoleMappings, TeamMappings};
use indexmap::IndexMap;

use super::GenericSsoConfig;
use super::failure::GenericSsoError;

// everything the exchange needs, resolved before the error wrapper starts
pub(super) struct SsoSetup {
    pub(super) settings: GenericSsoSettings,
    pub(super) role_mappings: Option<RoleMappings>,
    pub(super) team_mappings: Option<TeamMappings>,
    pub(super) headers: IndexMap<String, String>,
}

// the pinned order: variables, role mapping, team mapping, headers, each outside the error wrapper
pub(super) async fn load<Rows>(
    config: &GenericSsoConfig<'_>,
    rows: &Rows,
) -> Result<SsoSetup, GenericSsoError>
where
    Rows: SsoSettingsRows,
{
    let settings = generic_sso_settings(config.sources)?;
    let role_mappings = load_role_mappings(rows, config.sources).await?;
    let team_mappings = load_team_mappings(rows).await;
    let headers = decode_generic_headers(config.sources.sso_headers.as_deref().map(str::as_bytes))?;
    Ok(SsoSetup {
        settings,
        role_mappings,
        team_mappings,
        headers,
    })
}
