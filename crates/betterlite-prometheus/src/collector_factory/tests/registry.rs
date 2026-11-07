pub const PROMETHEUS_LOGGER_CLASS: &str = "PrometheusLogger";
pub const PROMETHEUS_CALLBACK_NAME: &str = "prometheus";

// one callback as the exporter's registry adapter sees it: B189's class name plus the logger it holds
#[derive(Clone, Debug)]
pub struct CallbackEntry<'callbacks, L> {
    pub class_name: String,
    pub logger: Option<&'callbacks L>,
}

// litellm.callbacks as the exporter keeps it, in registration order
#[derive(Clone, Debug)]
pub struct CallbackRegistry<'callbacks, L> {
    pub entries: Vec<CallbackEntry<'callbacks, L>>,
}

impl<'callbacks, L> CallbackRegistry<'callbacks, L> {
    #[must_use]
    pub fn get_instance(&self) -> Option<&'callbacks L> {
        for entry in &self.entries {
            let configured = entry.class_name == PROMETHEUS_CALLBACK_NAME
                || entry.class_name == PROMETHEUS_LOGGER_CLASS;
            if configured && let Some(logger) = entry.logger {
                return Some(logger);
            }
        }
        None
    }
}
