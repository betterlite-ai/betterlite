#[path = "tests/has_request_transforms.rs"]
mod has_request_transforms;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallbackKind {
    Builtin,
    Custom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestHook {
    PreCall,
    PreRequest,
    PreCallDeployment,
}

// A logger as the callback manager knows it: built-in or custom, and which request hooks it replaces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoggerTransforms<'logger> {
    pub kind: CallbackKind,
    pub overridden_hooks: &'logger [RequestHook],
}

// Ports has_request_transforms: a custom logger that overrides any request hook may rewrite the prompt.
#[must_use]
pub fn has_request_transforms(loggers: &[LoggerTransforms<'_>]) -> bool {
    loggers
        .iter()
        .any(|logger| logger.kind == CallbackKind::Custom && !logger.overridden_hooks.is_empty())
}
