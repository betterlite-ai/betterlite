use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;

use super::backend_construction::BackendConstructionError;
use super::cache::Cache;
use super::callback_registration::CacheCallbacks;
use super::settings::{CacheDependencies, CacheSettings};
use super::type_not_supported::wire_name;

// What LiteLLM keeps on its module: the current cache, the callback lists and print_verbose's hooks
pub struct CacheSwitches<L: LoggerSink, C: ConsoleSink> {
    pub cache: Option<Cache>,
    pub callbacks: CacheCallbacks,
    pub hooks: VerboseHooks<L, C>,
}

// The three arguments enable_cache and update_cache build a cache from
pub struct CacheSetup<'setup> {
    pub settings: CacheSettings,
    pub environment: &'setup (dyn EnvironmentLookup + Sync),
    pub dependencies: &'setup CacheDependencies,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheSwitch {
    Enable,
    Update,
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
        self.switch_cache(CacheSwitch::Enable, setup).await?;
        Ok(())
    }

    pub async fn update_cache(
        &mut self,
        setup: CacheSetup<'_>,
    ) -> Result<(), BackendConstructionError> {
        self.switch_cache(CacheSwitch::Update, setup).await?;
        Ok(())
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

    async fn switch_cache(
        &mut self,
        switch: CacheSwitch,
        setup: CacheSetup<'_>,
    ) -> Result<(), BackendConstructionError> {
        let (starting, finished) = match switch {
            CacheSwitch::Enable => ("LiteLLM: Enabling Cache", "LiteLLM: Cache enabled"),
            CacheSwitch::Update => ("LiteLLM: Updating Cache", "LiteLLM: Cache Updated"),
        };
        let _printed = self
            .hooks
            .main_print_verbose(Payload::String(starting.to_owned()));
        if switch == CacheSwitch::Enable {
            self.callbacks.register_cache();
        }
        if switch == CacheSwitch::Update || self.cache.is_none() {
            let cache = Cache::open(
                setup.settings,
                &mut self.callbacks,
                setup.environment,
                setup.dependencies,
            )
            .await?;
            self.cache = Some(cache);
        }
        self.announce(finished);
        Ok(())
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
