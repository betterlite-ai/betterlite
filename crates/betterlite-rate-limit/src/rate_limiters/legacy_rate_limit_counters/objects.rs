use super::usage_cache::LegacyCacheValue;

// CacheObject: the six values one batch read answers, each None when its key was None or nothing was stored
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheObject {
    pub current_global_requests: Option<LegacyCacheValue>,
    pub request_count_api_key: Option<LegacyCacheValue>,
    pub request_count_api_key_model: Option<LegacyCacheValue>,
    pub request_count_user_id: Option<LegacyCacheValue>,
    pub request_count_team_id: Option<LegacyCacheValue>,
    pub request_count_end_user_id: Option<LegacyCacheValue>,
}

// get_all_cache_objects' six arguments, each Some exactly when LiteLLM builds that key
#[derive(Clone, Default, Eq, PartialEq)]
pub struct CacheObjectKeys {
    pub current_global_requests: Option<String>,
    pub request_count_api_key: Option<String>,
    pub request_count_api_key_model: Option<String>,
    pub request_count_user_id: Option<String>,
    pub request_count_team_id: Option<String>,
    pub request_count_end_user_id: Option<String>,
}

impl CacheObjectKeys {
    #[must_use]
    pub fn into_list(self) -> [Option<String>; 6] {
        [
            self.current_global_requests,
            self.request_count_api_key,
            self.request_count_api_key_model,
            self.request_count_user_id,
            self.request_count_team_id,
            self.request_count_end_user_id,
        ]
    }
}

impl CacheObject {
    #[must_use]
    pub fn from_results(results: Vec<Option<LegacyCacheValue>>) -> Self {
        let mut values = results.into_iter();
        Self {
            current_global_requests: values.next().flatten(),
            request_count_api_key: values.next().flatten(),
            request_count_api_key_model: values.next().flatten(),
            request_count_user_id: values.next().flatten(),
            request_count_team_id: values.next().flatten(),
            request_count_end_user_id: values.next().flatten(),
        }
    }
}
