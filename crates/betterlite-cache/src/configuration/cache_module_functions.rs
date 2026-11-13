use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;

use super::backend_construction::BackendConstructionError;
use super::cache::Cache;
use super::callback_registration::CacheCallbacks;
use super::settings::{CacheDependencies, CacheSettings};
use super::type_not_supported::wire_name;

pub struct CacheSwitches<L: LoggerSink, C: ConsoleSink> {
    pub cache: Option<Cache>,
    pub callbacks: CacheCallbacks,
    pub hooks: VerboseHooks<L, C>,
}

pub struct CacheSetup<'setup> {
    pub settings: CacheSettings,
    pub environment: &'setup (dyn EnvironmentLookup + Sync),
    pub dependencies: &'setup CacheDependencies,
}

impl<L, C> CacheSwitches<L, C>
where
    L: LoggerSink + Send + Sync,
    C: ConsoleSink + Send + Sync,
{
    pub async fn enable_cache(
        &mut self,
        setup: CacheSetup<'_>,
    ) -> Result<(), BackendConstructionError> {
        let _enabling = self
            .hooks
            .main_print_verbose(Payload::String("LiteLLM: Enabling Cache".to_owned()));
        self.callbacks.register_cache();
        if self.cache.is_none() {
            let cache = Cache::open(
                setup.settings,
                &mut self.callbacks,
                setup.environment,
                setup.dependencies,
            )
            .await?;
            self.cache = Some(cache);
        }
        self.announce("LiteLLM: Cache enabled");
        Ok(())
    }

    pub async fn update_cache(
        &mut self,
        setup: CacheSetup<'_>,
    ) -> Result<Option<Cache>, BackendConstructionError> {
        let _updating = self
            .hooks
            .main_print_verbose(Payload::String("LiteLLM: Updating Cache".to_owned()));
        let cache = Cache::open(
            setup.settings,
            &mut self.callbacks,
            setup.environment,
            setup.dependencies,
        )
        .await?;
        let previous = self.cache.replace(cache);
        self.announce("LiteLLM: Cache Updated");
        Ok(previous)
    }

    pub fn disable_cache(&mut self) {
        let _disabling = self
            .hooks
            .main_print_verbose(Payload::String("LiteLLM: Disabling Cache".to_owned()));
        self.callbacks.unregister_cache();
        self.cache = None;
        let _disabled = self
            .hooks
            .main_print_verbose(Payload::String("LiteLLM: Cache disabled".to_owned()));
    }

    fn announce(&self, status: &str) {
        let description = self
            .cache
            .as_ref()
            .map_or_else(|| "none".to_owned(), cache_summary);
        let _printed = self
            .hooks
            .main_print_verbose(Payload::String(format!("{status}, {description}")));
    }
}

fn cache_summary(cache: &Cache) -> String {
    format!(
        "type={}, namespace={:?}, ttl={:?}, mode={:?}, supported_call_types={}",
        wire_name(cache.cache_type),
        cache.namespace,
        cache.ttl,
        cache.mode,
        cache.supported_call_types.len()
    )
}
