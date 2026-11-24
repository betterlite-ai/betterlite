use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::backend::AuthCacheTier;
use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use serde::Deserialize;

use super::assignment::{MembershipChange, groups_with_added, removed_groups};
use super::error::MemberEffectsError;

pub type CachedTeam = LiteLLMTeamTableCachedObj;

pub type StoreResult = Result<(), MemberEffectsError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheTarget {
    Team,
    Key,
}

// the four B733/B734/B737 entry points the patch helpers need, with their errors in this module's type
pub trait MemberEffectCaches: Send + Sync {
    fn read_team(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<CachedTeam>, MemberEffectsError>> + Send;

    fn store_team(
        &self,
        team_id: &str,
        team: &mut CachedTeam,
    ) -> impl Future<Output = StoreResult> + Send;

    fn read_key(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<UserAPIKeyAuth>, MemberEffectsError>> + Send;

    fn store_key(
        &self,
        token: &str,
        auth: &mut UserAPIKeyAuth,
    ) -> impl Future<Output = StoreResult> + Send;
}

pub struct GatewayMemberCaches<Tier: AuthCacheTier> {
    pub cache: Arc<AuthCache<Tier>>,
    pub invalidation: Arc<AuthInvalidation>,
}

impl<Tier: AuthCacheTier> MemberEffectCaches for GatewayMemberCaches<Tier> {
    async fn read_team(&self, key: &str) -> Result<Option<CachedTeam>, MemberEffectsError> {
        self.cache
            .async_get_typed::<CachedTeam>(key, false)
            .await
            .map_err(MemberEffectsError::TeamPayload)
    }

    async fn store_team(&self, team_id: &str, team: &mut CachedTeam) -> StoreResult {
        self.invalidation
            .cache_team_object(team_id, team)
            .await
            .map_err(MemberEffectsError::CacheWrite)
    }

    async fn read_key(&self, token: &str) -> Result<Option<UserAPIKeyAuth>, MemberEffectsError> {
        self.cache
            .async_get_auth(token, false)
            .await
            .map_err(MemberEffectsError::KeyAuth)
    }

    async fn store_key(&self, token: &str, auth: &mut UserAPIKeyAuth) -> StoreResult {
        self.invalidation
            .cache_key_object(token, auth)
            .await
            .map_err(MemberEffectsError::CacheWrite)
    }
}

trait CachePatchTarget {
    type Object;

    fn cache_key(id: &str) -> String;

    fn read(
        &self,
        cache_key: &str,
    ) -> impl Future<Output = Result<Option<Self::Object>, MemberEffectsError>> + Send;

    fn store(
        &self,
        id: &str,
        object: &mut Self::Object,
    ) -> impl Future<Output = Result<(), MemberEffectsError>> + Send;

    fn groups(object: &Self::Object) -> Result<Option<Vec<String>>, MemberEffectsError>;

    fn with_groups(
        object: &Self::Object,
        groups: Vec<String>,
    ) -> Result<Self::Object, MemberEffectsError>;
}

struct TeamCaches<'cache, Cache: ?Sized>(&'cache Cache);

struct KeyCaches<'cache, Cache: ?Sized>(&'cache Cache);

impl<Cache: MemberEffectCaches + ?Sized> CachePatchTarget for TeamCaches<'_, Cache> {
    type Object = LiteLLMTeamTableCachedObj;

    fn cache_key(id: &str) -> String {
        format!("team_id:{id}")
    }

    async fn read(&self, cache_key: &str) -> Result<Option<Self::Object>, MemberEffectsError> {
        self.0.read_team(cache_key).await
    }

    async fn store(&self, id: &str, object: &mut Self::Object) -> Result<(), MemberEffectsError> {
        self.0.store_team(id, object).await
    }

    fn groups(object: &Self::Object) -> Result<Option<Vec<String>>, MemberEffectsError> {
        Ok(object.team.fields.base.access_group_ids.clone())
    }

    fn with_groups(
        object: &Self::Object,
        groups: Vec<String>,
    ) -> Result<Self::Object, MemberEffectsError> {
        let mut patched = object.clone();
        patched.team.fields.base.access_group_ids = Some(groups);
        Ok(patched)
    }
}

impl<Cache: MemberEffectCaches + ?Sized> CachePatchTarget for KeyCaches<'_, Cache> {
    type Object = UserAPIKeyAuth;

    fn cache_key(id: &str) -> String {
        id.to_owned()
    }

    async fn read(&self, cache_key: &str) -> Result<Option<Self::Object>, MemberEffectsError> {
        self.0.read_key(cache_key).await
    }

    async fn store(&self, id: &str, object: &mut Self::Object) -> Result<(), MemberEffectsError> {
        self.0.store_key(id, object).await
    }

    fn groups(object: &Self::Object) -> Result<Option<Vec<String>>, MemberEffectsError> {
        key_groups(object)
    }

    fn with_groups(
        object: &Self::Object,
        groups: Vec<String>,
    ) -> Result<Self::Object, MemberEffectsError> {
        key_with_groups(object, groups)
    }
}

struct MembershipPatch<'patch> {
    ids: &'patch [String],
    access_group_id: &'patch str,
    change: MembershipChange,
}

pub async fn change_cached_membership<Cache>(
    cache: &Cache,
    target: CacheTarget,
    ids: &[String],
    access_group_id: &str,
    change: MembershipChange,
) -> Result<(), MemberEffectsError>
where
    Cache: MemberEffectCaches + ?Sized,
{
    let patch = MembershipPatch {
        ids,
        access_group_id,
        change,
    };
    match target {
        CacheTarget::Team => patch_cached(&TeamCaches(cache), &patch).await,
        CacheTarget::Key => patch_cached(&KeyCaches(cache), &patch).await,
    }
}

async fn patch_cached<Target>(
    target: &Target,
    patch: &MembershipPatch<'_>,
) -> Result<(), MemberEffectsError>
where
    Target: CachePatchTarget + Send + Sync + ?Sized,
    Target::Object: Send,
{
    for id in patch.ids {
        let Some(object) = target.read(&Target::cache_key(id)).await? else {
            continue;
        };
        let existing = Target::groups(&object)?;
        let Some(groups) = cached_groups_after(existing, patch.change, patch.access_group_id)
        else {
            continue;
        };
        let mut patched = Target::with_groups(&object, groups)?;
        target.store(id, &mut patched).await?;
    }
    Ok(())
}

// a null array is a miss for remove and [group] for add; remove rewrites even when the group is absent
fn cached_groups_after(
    existing: Option<Vec<String>>,
    change: MembershipChange,
    access_group_id: &str,
) -> Option<Vec<String>> {
    match change {
        MembershipChange::Add => groups_with_added(existing, access_group_id),
        MembershipChange::Remove => {
            let groups = existing.filter(|ids| !ids.is_empty())?;
            Some(removed_groups(groups, access_group_id))
        }
    }
}

#[derive(Deserialize)]
struct CachedKeyFields {
    #[serde(default)]
    access_group_ids: Option<Vec<String>>,
}

// UserAPIKeyAuth keeps its fields private, so the cached key is read through its serde form
fn key_groups(auth: &UserAPIKeyAuth) -> Result<Option<Vec<String>>, MemberEffectsError> {
    let fields: CachedKeyFields = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(MemberEffectsError::KeyJson)?;
    Ok(fields.access_group_ids)
}

fn key_with_groups(
    auth: &UserAPIKeyAuth,
    groups: Vec<String>,
) -> Result<UserAPIKeyAuth, MemberEffectsError> {
    let mut fields: BTreeMap<String, JsonData> = serde_json::to_value(auth)
        .and_then(serde_json::from_value)
        .map_err(MemberEffectsError::KeyJson)?;
    let listed: Vec<JsonData> = groups.into_iter().map(JsonData::String).collect();
    fields.insert("access_group_ids".to_owned(), JsonData::Array(listed));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(MemberEffectsError::KeyAuth)
}
