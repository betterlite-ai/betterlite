use std::error::Error as StdError;
use std::future::Future;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;

use crate::auth::object_permission_hydration::reads::ObjectPermission;

pub mod agent;
pub mod error;
pub mod live;
pub mod reads;

mod caller;
mod end_user;
mod org;
mod servers;
mod tests;
mod tools;
mod user;

use agent::AgentServer;
use error::{CeilingsError, CeilingsResult};
use reads::CeilingReads;
use tools::{ToolCall, ToolLevel};

// _get_allowed_mcp_servers_for_org and _get_allowed_mcp_servers_for_user answer both [] and None, which mean different things
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CeilingServers {
    NoRestriction,
    Unresolved,
    Servers(Vec<String>),
}

// the (list, restricts) pair every server ceiling answers; a later level may only cap a lower restriction
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capped {
    pub servers: Vec<String>,
    pub restricts: bool,
}

// a keyless admitted source is denied by an unresolved org ceiling, while a key keeps its list
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CeilingSource {
    Key,
    Keyless,
}

// the ceiling steps B4671 and B4672 chain, in the order LiteLLM applies them
pub trait CeilingLevels {
    type Source: StdError + Send + Sync + 'static;
    fn org_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<CeilingServers, CeilingsError<Self::Source>>> + Send;
    fn apply_primary_org_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
        has_lower_level_restrictions: bool,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Capped, CeilingsError<Self::Source>>> + Send;
    fn end_user_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Vec<String>, CeilingsError<Self::Source>>> + Send;
    fn user_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = CeilingServers> + Send;
    fn apply_user_server_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Capped, CeilingsError<Self::Source>>> + Send;
    fn user_places_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = bool> + Send;
    fn admin_view_unscoped(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = bool> + Send;
    fn agent_servers(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        prefetched: Option<&ObjectPermission>,
    ) -> impl Future<Output = Result<Vec<String>, CeilingsError<Self::Source>>> + Send;
    fn agent_access_group_ceiling(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    fn apply_agent_caller_ceiling(
        &self,
        allowed: Vec<String>,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Capped, CeilingsError<Self::Source>>> + Send;
    fn apply_user_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    fn apply_end_user_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    // ast-grep-ignore: no-long-names
    fn apply_agent_caller_tool_ceiling(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    // ast-grep-ignore: no-long-names
    fn agent_tool_permissions_for_server(
        &self,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        prefetched: Option<&ObjectPermission>,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
    // ast-grep-ignore: no-long-names
    fn apply_agent_and_org_tool_ceilings(
        &self,
        allowed: Option<Vec<String>>,
        server_id: &str,
        auth: Option<&UserAPIKeyAuth>,
        source_kind: CeilingSource,
    ) -> impl Future<Output = Result<Option<Vec<String>>, CeilingsError<Self::Source>>> + Send;
}

// the ceiling steps over one set of reads; every dependency arrives through the constructor
pub struct Ceilings<Reads: CeilingReads> {
    pub reads: Reads,
}

// every level below has one shape: the trait method carries LiteLLM's step name, and the bracket says what the level hands on
macro_rules! ceiling_levels {
    ($(async fn $name:ident(&self $(, $arg:ident : $ty:ty)*) -> $ret:ty = $target:path [ $($call:tt)* ];)*) => {
        impl<Reads> CeilingLevels for Ceilings<Reads>
        where
            Reads: CeilingReads,
        {
            type Source = Reads::Error;
            $(
                async fn $name(&self $(, $arg: $ty)*) -> $ret {
                    $target(self, $($call)*).await
                }
            )*
        }
    };
}

ceiling_levels! {
    async fn org_servers(&self, auth: Option<&UserAPIKeyAuth>) -> CeilingsResult<CeilingServers, Reads> = org::org_servers [auth];
    async fn apply_primary_org_ceiling(&self, allowed: Vec<String>, auth: Option<&UserAPIKeyAuth>, has_lower_level_restrictions: bool, source_kind: CeilingSource) -> CeilingsResult<Capped, Reads> = org::apply_primary_org_ceiling [allowed, auth, has_lower_level_restrictions, source_kind];
    async fn end_user_servers(&self, auth: Option<&UserAPIKeyAuth>) -> CeilingsResult<Vec<String>, Reads> = end_user::end_user_servers [auth];
    async fn user_servers(&self, auth: Option<&UserAPIKeyAuth>) -> CeilingServers = user::user_servers [auth];
    async fn apply_user_server_ceiling(&self, allowed: Vec<String>, auth: Option<&UserAPIKeyAuth>, source_kind: CeilingSource) -> CeilingsResult<Capped, Reads> = user::apply_user_server_ceiling [allowed, auth, source_kind];
    async fn user_places_ceiling(&self, auth: Option<&UserAPIKeyAuth>) -> bool = user::user_places_ceiling [auth];
    async fn admin_view_unscoped(&self, auth: Option<&UserAPIKeyAuth>) -> bool = user::admin_view_unscoped [auth];
    async fn agent_servers(&self, auth: Option<&UserAPIKeyAuth>, prefetched: Option<&ObjectPermission>) -> CeilingsResult<Vec<String>, Reads> = agent::agent_servers [auth, prefetched];
    async fn agent_access_group_ceiling(&self, auth: Option<&UserAPIKeyAuth>) -> CeilingsResult<Option<Vec<String>>, Reads> = agent::agent_access_group_ceiling [auth];
    async fn apply_agent_caller_ceiling(&self, allowed: Vec<String>, auth: Option<&UserAPIKeyAuth>) -> CeilingsResult<Capped, Reads> = caller::apply_agent_caller_ceiling [allowed, auth];
    async fn apply_user_tool_ceiling(&self, allowed: Option<Vec<String>>, server_id: &str, auth: Option<&UserAPIKeyAuth>, source_kind: CeilingSource) -> CeilingsResult<Option<Vec<String>>, Reads> = tools::apply_human_tool_ceiling [ToolCall { allowed, server_id, auth }, ToolLevel::User(source_kind)];
    async fn apply_end_user_tool_ceiling(&self, allowed: Option<Vec<String>>, server_id: &str, auth: Option<&UserAPIKeyAuth>) -> CeilingsResult<Option<Vec<String>>, Reads> = tools::apply_human_tool_ceiling [ToolCall { allowed, server_id, auth }, ToolLevel::EndUser];
    // ast-grep-ignore: no-long-names
    async fn apply_agent_caller_tool_ceiling(&self, allowed: Option<Vec<String>>, server_id: &str, auth: Option<&UserAPIKeyAuth>) -> CeilingsResult<Option<Vec<String>>, Reads> = tools::apply_caller_tools [ToolCall { allowed, server_id, auth }];
    // ast-grep-ignore: no-long-names
    async fn agent_tool_permissions_for_server(&self, server_id: &str, auth: Option<&UserAPIKeyAuth>, prefetched: Option<&ObjectPermission>) -> CeilingsResult<Option<Vec<String>>, Reads> = agent::agent_server_tools [AgentServer { server_id, auth, prefetched }];
    // ast-grep-ignore: no-long-names
    async fn apply_agent_and_org_tool_ceilings(&self, allowed: Option<Vec<String>>, server_id: &str, auth: Option<&UserAPIKeyAuth>, source_kind: CeilingSource) -> CeilingsResult<Option<Vec<String>>, Reads> = tools::apply_agent_org_tools [ToolCall { allowed, server_id, auth }, source_kind];
}
