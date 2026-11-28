use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use betterlite_db::mcp_user_env_vars::error::UserEnvVarsError;
use betterlite_db::mcp_user_env_vars::store::UserEnvVars;

// B4714's store as these routes read and write it; the mount task delegates each call one to one
#[async_trait]
pub trait UserEnvVarsStore: Send + Sync {
    async fn get_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<UserEnvVars, UserEnvVarsError>;

    async fn get_user_env_vars_bulk(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> Result<BTreeMap<String, UserEnvVars>, UserEnvVarsError>;

    async fn merge_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
        updates: &UserEnvVars,
        allowed_names: &BTreeSet<String>,
    ) -> Result<UserEnvVars, UserEnvVarsError>;

    async fn delete_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<(), UserEnvVarsError>;
}

// LiteLLM's invalidate_user_env_vars_cache: one entry dropped after a write, never before it
pub trait UserEnvVarsCache: Send + Sync {
    fn invalidate(&self, user_id: &str, server_id: &str);
}
