use std::error::Error as StdError;

use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_cache::backend::AuthCacheTier;

use super::data::ManagementData;

// LiteLLM's synchronous user_api_key_cache.delete_cache; B733 routes a key to its own partition
pub trait ManagementCacheEviction: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn delete_cache(&self, key: &str) -> Result<(), Self::Error>;
}

impl<Tier> ManagementCacheEviction for AuthCache<Tier>
where
    Tier: AuthCacheTier,
{
    type Error = AuthCacheError;

    fn delete_cache(&self, key: &str) -> Result<(), Self::Error> {
        self.tier(key).delete(key)?;
        Ok(())
    }
}

fn delete_each<Cache>(cache: &Cache, keys: &[String]) -> Result<(), Cache::Error>
where
    Cache: ManagementCacheEviction,
{
    for key in keys {
        cache.delete_cache(key)?;
    }
    Ok(())
}

// LiteLLM's _delete_api_key_from_cache: an update deletes its one key, a delete lists the keys
pub fn delete_api_key_from_cache<Cache>(
    cache: &Cache,
    data: Option<&ManagementData<'_>>,
) -> Result<(), Cache::Error>
where
    Cache: ManagementCacheEviction,
{
    match data {
        Some(ManagementData::UpdateKey(key)) => cache.delete_cache(key),
        Some(ManagementData::DeleteKeys(keys)) => delete_each(cache, keys),
        _ => Ok(()),
    }
}

// LiteLLM's _delete_user_id_from_cache
pub fn delete_user_id_from_cache<Cache>(
    cache: &Cache,
    data: Option<&ManagementData<'_>>,
) -> Result<(), Cache::Error>
where
    Cache: ManagementCacheEviction,
{
    match data {
        Some(ManagementData::UpdateUser(user_id)) => cache.delete_cache(user_id),
        Some(ManagementData::DeleteUsers(user_ids)) => delete_each(cache, user_ids),
        _ => Ok(()),
    }
}

// LiteLLM's _delete_team_id_from_cache
pub fn delete_team_id_from_cache<Cache>(
    cache: &Cache,
    data: Option<&ManagementData<'_>>,
) -> Result<(), Cache::Error>
where
    Cache: ManagementCacheEviction,
{
    match data {
        Some(ManagementData::UpdateTeam(team_id)) => cache.delete_cache(team_id),
        Some(ManagementData::DeleteTeams(team_ids)) => delete_each(cache, team_ids),
        _ => Ok(()),
    }
}

// LiteLLM's _delete_customer_id_from_cache
pub fn delete_customer_id_from_cache<Cache>(
    cache: &Cache,
    data: Option<&ManagementData<'_>>,
) -> Result<(), Cache::Error>
where
    Cache: ManagementCacheEviction,
{
    match data {
        Some(ManagementData::UpdateCustomer(user_id)) => cache.delete_cache(user_id),
        Some(ManagementData::DeleteCustomers(user_ids)) => delete_each(cache, user_ids),
        _ => Ok(()),
    }
}
