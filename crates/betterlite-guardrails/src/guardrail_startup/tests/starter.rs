struct NamedLogger {
    name: String,
}

impl CallbackLogger for NamedLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

struct EveryName;

impl LoggerResolver for EveryName {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, LoggingRegistryError> {
        let logger = NamedLogger {
            name: name.to_owned(),
        };
        Ok(Some(Arc::new(logger)))
    }
}

const FLAGS: FeatureFlags = FeatureFlags {
    otel_v2_enabled: false,
};

fn general_names(registry: &CallbackRegistry) -> Vec<String> {
    registry
        .list_identities(CallbackList::Callbacks)
        .iter()
        .map(|identity| identity.class_name.clone())
        .collect()
}

#[test]
fn legacy_starter_registers_callbacks() {
    let mut registry = CallbackRegistry::new(100).with_resolver(Arc::new(EveryName));
    let mut starter = RegistryStarter {
        registry: &mut registry,
        flags: FLAGS,
    };
    assert!(!starter.is_started("presidio"));
    let names = ["presidio".to_owned(), "lakera_prompt_injection".to_owned()];
    starter.start(&names, &CallbackArgs::new()).unwrap();
    assert!(starter.is_started("presidio"));
    assert!(starter.is_started("lakera_prompt_injection"));
    assert!(!starter.is_started("other"));
    // a second start finds both registered and still succeeds
    starter.start(&names, &CallbackArgs::new()).unwrap();
    assert_eq!(
        general_names(&registry),
        ["presidio", "lakera_prompt_injection"]
    );
}

#[test]
fn legacy_starter_only_counts_the_general_callback_list() {
    let mut registry = CallbackRegistry::new(100).with_resolver(Arc::new(EveryName));
    registry
        .register_configured(
            "presidio",
            &CallbackVars::new(),
            CallbackList::Success,
            FLAGS,
        )
        .unwrap();
    let starter = RegistryStarter {
        registry: &mut registry,
        flags: FLAGS,
    };
    assert!(!starter.is_started("presidio"));
}

#[test]
fn legacy_starter_start_registers_in_the_general_list() {
    let mut registry = CallbackRegistry::new(100).with_resolver(Arc::new(EveryName));
    let mut starter = RegistryStarter {
        registry: &mut registry,
        flags: FLAGS,
    };
    starter
        .start(&["presidio".to_owned()], &CallbackArgs::new())
        .unwrap();
    assert!(registry.list_identities(CallbackList::Success).is_empty());
    assert_eq!(general_names(&registry), ["presidio"]);
}

#[test]
fn legacy_starter_maps_a_registry_error_to_a_legacy_error() {
    let mut registry = CallbackRegistry::new(100);
    let mut starter = RegistryStarter {
        registry: &mut registry,
        flags: FLAGS,
    };
    let failed = starter
        .start(&["presidio".to_owned()], &CallbackArgs::new())
        .unwrap_err();
    assert!(matches!(
        &failed,
        StartupError::Legacy { message } if message == "no callback adapter is installed for 'presidio'"
    ));
}

#[test]
fn legacy_starter_treats_a_full_list_as_success() {
    let mut registry = CallbackRegistry::new(0).with_resolver(Arc::new(EveryName));
    let mut starter = RegistryStarter {
        registry: &mut registry,
        flags: FLAGS,
    };
    starter
        .start(&["presidio".to_owned()], &CallbackArgs::new())
        .unwrap();
    assert!(!starter.is_started("presidio"));
}
