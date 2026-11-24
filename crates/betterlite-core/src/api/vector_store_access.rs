pub mod auth;
pub mod decode;
pub mod error;
pub mod grants;
pub mod store;

mod tests;

use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::ui_session_identity::UiSessionTeams;

use self::auth::ObjectPermissionLookup;

// proxy/vector_store_endpoints/utils.py's managed-store grants over the caller's auth rows and lookups
pub struct VectorStoreAccess<'access, Permissions, Teams, SessionTeams>
where
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    pub permissions: &'access Permissions,
    pub teams: &'access Teams,
    pub session_teams: &'access SessionTeams,
}
