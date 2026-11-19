use std::fmt::Display;

use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::error::HookRegistryError;
use super::kind::HookKind;
use super::spec::{HookSettings, HookSpec};

// The wire name and settings every arm of one kind's dispatch needs, so each arm line stays short
pub(super) struct Arms<'a> {
    pub(super) kind: &'a str,
    pub(super) settings: &'a HookSettings,
}

impl Arms<'_> {
    pub(super) fn take<A, E, F>(
        &self,
        result: Result<A, E>,
        wrap: F,
    ) -> Result<HookSpec, HookRegistryError>
    where
        E: Display,
        F: FnOnce(Box<A>) -> HookKind,
    {
        let args = result.map_err(|error| HookRegistryError::init(self.kind, error))?;
        Ok(HookSpec::new(self.settings, wrap(Box::new(args))))
    }

    pub(super) fn plain<A, F>(&self, value: A, wrap: F) -> HookSpec
    where
        F: FnOnce(Box<A>) -> HookKind,
    {
        HookSpec::new(self.settings, wrap(Box::new(value)))
    }

    // the class tasks parse these params themselves, so the spec keeps the params
    pub(super) fn class<F>(&self, params: &LitellmParams, wrap: F) -> HookSpec
    where
        F: FnOnce(Box<LitellmParams>) -> HookKind,
    {
        HookSpec::new(self.settings, wrap(Box::new(params.clone())))
    }
}
