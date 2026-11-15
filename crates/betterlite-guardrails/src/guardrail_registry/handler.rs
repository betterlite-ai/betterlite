use std::borrow::Cow;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::build::GuardrailFactories;
use super::callback::{CallbackSpec, GuardrailCallback};
use super::changed::params_changed;
use super::error::RegistryError;
use super::integrations::compression_kind;
use super::scoping::configure_scoping;
use super::stable_id::stable_guardrail_id;

const MISSING_ENV: &str = "None";
const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Clone, Copy, Default)]
pub struct HandlerSettings {
    pub strict_modes: bool,
    pub global_skip_tool_message: bool,
}

struct CallbackSlots {
    primary: Option<GuardrailCallback>,
    siblings: Vec<GuardrailCallback>,
}

pub struct InMemoryGuardrailHandler {
    factories: GuardrailFactories,
    env: Box<dyn EnvironmentLookup + Send + Sync>,
    settings: HandlerSettings,
    guardrails: IndexMap<String, Guardrail>,
    callbacks: IndexMap<String, CallbackSlots>,
    sources: IndexMap<String, GuardrailDefinitionLocation>,
}

impl InMemoryGuardrailHandler {
    #[must_use]
    pub fn new(
        factories: GuardrailFactories,
        env: Box<dyn EnvironmentLookup + Send + Sync>,
        settings: HandlerSettings,
    ) -> Self {
        Self {
            factories,
            env,
            settings,
            guardrails: IndexMap::new(),
            callbacks: IndexMap::new(),
            sources: IndexMap::new(),
        }
    }

    pub fn initialize_guardrail(
        &mut self,
        mut guardrail: Guardrail,
        source: GuardrailDefinitionLocation,
    ) -> Result<Guardrail, RegistryError> {
        let name = guardrail.guardrail_name.clone();
        let guardrail_id = guardrail
            .guardrail_id
            .clone()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| {
                stable_guardrail_id(&name, &|candidate: &str| {
                    self.guardrails.contains_key(candidate)
                })
            });
        guardrail.guardrail_id = Some(guardrail_id.clone());
        if let Some(held) = self.guardrails.get(&guardrail_id).cloned() {
            // LiteLLM honors the caller's source even on the early-return path
            self.sources.insert(guardrail_id, source);
            return Ok(held);
        }
        resolve_env(&mut guardrail.litellm_params, self.env.as_ref());
        let callbacks = self.build_callbacks(&guardrail)?;
        let parsed = parsed_guardrail(&guardrail_id, &guardrail);
        self.remember(guardrail_id, callbacks, parsed.clone(), source);
        Ok(parsed)
    }

    pub fn update_in_memory_guardrail(
        &mut self,
        guardrail_id: &str,
        mut guardrail: Guardrail,
        source: GuardrailDefinitionLocation,
    ) -> Result<(), RegistryError> {
        guardrail.guardrail_id = Some(guardrail_id.to_owned());
        let existing = self.guardrails.get(guardrail_id);
        if params_changed(existing, &guardrail)? {
            self.reinitialize_guardrail(&guardrail, source)?;
            return Ok(());
        }
        self.guardrails.insert(guardrail_id.to_owned(), guardrail);
        self.sources.insert(guardrail_id.to_owned(), source);
        Ok(())
    }

    pub fn reinitialize_guardrail(
        &mut self,
        guardrail: &Guardrail,
        source: GuardrailDefinitionLocation,
    ) -> Result<Option<Guardrail>, RegistryError> {
        let guardrail_id = guardrail.guardrail_id.clone().filter(|id| !id.is_empty());
        let Some(held_id) = guardrail_id else {
            tracing::error!("Cannot reinitialize guardrail without guardrail_id");
            return Ok(None);
        };
        let previous = self.guardrails.get(&held_id).cloned();
        let previous_source = self.sources.get(&held_id).copied().unwrap_or(source);
        self.delete_in_memory_guardrail(&held_id);
        match self.initialize_guardrail(guardrail.clone(), source) {
            Ok(initialized) => Ok(Some(initialized)),
            Err(error) => {
                self.restore_guardrail(&held_id, previous, previous_source, &error);
                Err(RegistryError::InitFailed {
                    source: Box::new(error),
                })
            }
        }
    }

    pub fn delete_in_memory_guardrail(&mut self, guardrail_id: &str) {
        drop(self.guardrails.shift_remove(guardrail_id));
        self.sources.shift_remove(guardrail_id);
        drop(self.callbacks.shift_remove(guardrail_id));
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn list_in_memory_guardrails(&self) -> Vec<Guardrail> {
        self.guardrails.values().cloned().collect()
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn get_guardrail_by_id(&self, guardrail_id: &str) -> Option<&Guardrail> {
        self.guardrails.get(guardrail_id)
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn get_source(&self, guardrail_id: &str) -> Option<GuardrailDefinitionLocation> {
        self.sources.get(guardrail_id).copied()
    }

    #[must_use]
    pub fn list_config_guardrails(&self) -> Vec<Guardrail> {
        self.guardrails
            .iter()
            .filter(|(id, _guardrail)| {
                self.sources.get(*id) == Some(&GuardrailDefinitionLocation::Config)
            })
            .map(|(_id, guardrail)| guardrail.clone())
            .collect()
    }

    #[must_use]
    pub fn get_config_guardrail_by_id(&self, guardrail_id: &str) -> Option<&Guardrail> {
        if self.sources.get(guardrail_id) != Some(&GuardrailDefinitionLocation::Config) {
            return None;
        }
        self.guardrails.get(guardrail_id)
    }

    pub fn all_callbacks(&self) -> impl Iterator<Item = &GuardrailCallback> {
        let primaries = self
            .callbacks
            .values()
            .filter_map(|slots| slots.primary.as_ref());
        let siblings = self
            .callbacks
            .values()
            .flat_map(|slots| slots.siblings.iter());
        primaries.chain(siblings)
    }

    fn build_callbacks(
        &self,
        guardrail: &Guardrail,
    ) -> Result<Vec<GuardrailCallback>, RegistryError> {
        let params = &guardrail.litellm_params;
        let kind = params.guardrail.as_str();
        let mut callbacks = self.factories.build(
            kind,
            params,
            &guardrail.guardrail_name,
            self.settings.strict_modes,
        )?;
        mark_compression(&mut callbacks, kind);
        for callback in &mut callbacks {
            configure_scoping(
                callback,
                &guardrail.guardrail_name,
                params,
                self.settings.global_skip_tool_message,
            )?;
        }
        Ok(callbacks)
    }

    fn remember(
        &mut self,
        guardrail_id: String,
        callbacks: Vec<GuardrailCallback>,
        guardrail: Guardrail,
        source: GuardrailDefinitionLocation,
    ) {
        let mut remaining = callbacks.into_iter();
        let slots = CallbackSlots {
            primary: remaining.next(),
            siblings: remaining.collect(),
        };
        self.guardrails.insert(guardrail_id.clone(), guardrail);
        self.callbacks.insert(guardrail_id.clone(), slots);
        self.sources.insert(guardrail_id, source);
    }

    fn restore_guardrail(
        &mut self,
        guardrail_id: &str,
        previous: Option<Guardrail>,
        previous_source: GuardrailDefinitionLocation,
        error: &RegistryError,
    ) {
        let Some(previous_guardrail) = previous else {
            return;
        };
        tracing::error!(
            %guardrail_id,
            %error,
            "Reinitializing guardrail with updated params failed; restoring the previous configuration"
        );
        if let Err(restore) = self.initialize_guardrail(previous_guardrail, previous_source) {
            tracing::error!(%guardrail_id, %restore, "Restoring previous guardrail also failed");
        }
    }
}

fn parsed_guardrail(guardrail_id: &str, guardrail: &Guardrail) -> Guardrail {
    Guardrail {
        guardrail_id: Some(guardrail_id.to_owned()),
        guardrail_name: guardrail.guardrail_name.clone(),
        litellm_params: guardrail.litellm_params.clone(),
        guardrail_info: guardrail.guardrail_info.clone(),
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    }
}

// the registry sets the kind itself, so no compression task has to
pub(super) fn mark_compression(callbacks: &mut [GuardrailCallback], provider: &str) {
    let Some(kind) = compression_kind(provider) else {
        return;
    };
    for callback in callbacks {
        callback.spec = CallbackSpec::Compression(kind);
    }
}

fn resolve_env(params: &mut LitellmParams, env: &dyn EnvironmentLookup) {
    let fields = &mut params.base.fields;
    if let Some(current) = fields
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned())
        && let Cow::Owned(secret) = resolved_value(&current, env)
    {
        fields.api_key = Some(SecretString::from(secret));
    }
    if let Some(current) = fields.api_base.clone()
        && let Cow::Owned(base) = resolved_value(&current, env)
    {
        fields.api_base = Some(base);
    }
}

fn resolved_value<'reference>(
    value: &'reference str,
    env: &dyn EnvironmentLookup,
) -> Cow<'reference, str> {
    let Some(name) = value.strip_prefix(ENVIRONMENT_PREFIX) else {
        return Cow::Borrowed(value);
    };
    let resolved = env.lookup(name).unwrap_or_else(|| MISSING_ENV.to_owned());
    Cow::Owned(resolved)
}
