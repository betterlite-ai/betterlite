pub const CACHE_CALLBACK: &str = "cache";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheCallbacks {
    pub input_callback: Vec<String>,
    pub success_callback: Vec<String>,
    pub async_success_callback: Vec<String>,
}

impl CacheCallbacks {
    pub fn register_cache(&mut self) {
        add_cache_callback(&mut self.input_callback);
        add_cache_callback(&mut self.success_callback);
        add_cache_callback(&mut self.async_success_callback);
    }

    pub fn unregister_cache(&mut self) {
        remove_cache_callback(&mut self.input_callback);
        remove_cache_callback(&mut self.success_callback);
        remove_cache_callback(&mut self.async_success_callback);
    }
}

fn add_cache_callback(list: &mut Vec<String>) {
    if !list.iter().any(|name| name == CACHE_CALLBACK) {
        list.push(CACHE_CALLBACK.to_owned());
    }
}

// LiteLLM's list.remove takes the first match and raises when there is none, which is ignored
fn remove_cache_callback(list: &mut Vec<String>) {
    if let Some(index) = list.iter().position(|name| name == CACHE_CALLBACK) {
        list.remove(index);
    }
}
