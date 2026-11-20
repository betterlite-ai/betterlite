use std::fmt::Display;

use super::error::HookRegistryError;
use super::kind::HookKind;
use super::spec::{HookSettings, HookSpec};

// The wire name and settings every arm of one kind's dispatch needs, so each arm line stays short
pub(super) struct Arms<'settings> {
    pub(super) kind: &'settings str,
    pub(super) settings: &'settings HookSettings,
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
        result
            .map_err(|error| HookRegistryError::init(self.kind, error))
            .map(|args| HookSpec::new(self.settings, wrap(Box::new(args))))
    }
}
