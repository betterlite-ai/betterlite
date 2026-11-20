use std::future::Future;

use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_invalidation::transport::InvalidationTransport;

use super::statement::{AllowlistKind, TouchedRow};

// B733's cache and B734's publish, behind one port so the statement tests can record the calls.
pub trait AllowlistCache: Send + Sync {
    fn evict_and_broadcast(
        &self,
        cache_keys: &[String],
    ) -> impl Future<Output = Result<(), InvalidationError>> + Send;
}

// B734 treats an eviction failure as best effort: it warns, publishes anyway and reports no error.
impl AllowlistCache for AuthInvalidation {
    async fn evict_and_broadcast(&self, cache_keys: &[String]) -> Result<(), InvalidationError> {
        InvalidationTransport::evict_and_broadcast(self.transport.as_ref(), cache_keys, self).await;
        Ok(())
    }
}

// The five pinned key shapes, in the row's kind and with no deduplication or sorting.
#[must_use]
pub fn cache_keys(row: &TouchedRow) -> Vec<String> {
    match row.kind {
        AllowlistKind::Team => {
            let object_id = row.object_id.as_str();
            let mut keys = vec![format!("team_id:{object_id}")];
            if let Some(alias) = row.team_alias.as_deref().filter(|alias| !alias.is_empty()) {
                keys.push(format!("team_alias:{alias}"));
            }
            keys
        }
        AllowlistKind::Key | AllowlistKind::User => vec![row.object_id.clone()],
        AllowlistKind::Organization => {
            let object_id = row.object_id.as_str();
            vec![
                format!("org_id:{object_id}"),
                format!("org_id:{object_id}:with_budget"),
            ]
        }
        AllowlistKind::Project => {
            let object_id = row.object_id.as_str();
            vec![format!("project_id:{object_id}")]
        }
    }
}
