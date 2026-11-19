use betterlite_router::complexity_config::error::ComplexityConfigError;

#[derive(Debug, thiserror::Error)]
pub enum StrategyRouterWriteError {
    #[error(
        "litellm_params.model='{model}' does not start with 'auto_router/' but the deployment \
         carries auto-router settings ({settings}), so the router could not load it. Keep the \
         'auto_router/' prefix; to change the name clients call, edit the public model_name instead."
    )]
    UnexpectedSettings { model: String, settings: String },
    #[error(
        "litellm_params.model='{model}' repeats the 'auto_router/' prefix, so the router could not \
         load it. Use '{remainder}'; to change the name clients call, edit the public model_name \
         instead."
    )]
    RepeatedPrefix { model: String, remainder: String },
    #[error(
        "litellm_params.model='{model}' is missing the router name after the 'auto_router/' prefix."
    )]
    MissingRouterName { model: String },
    #[error(
        "litellm_params.model='{model}' selects the {kind} router, which requires {requirements} \
         in litellm_params."
    )]
    MissingSettings {
        model: String,
        kind: &'static str,
        requirements: String,
    },
}

#[derive(Debug, thiserror::Error)]
#[error(
    "litellm_params sets complexity_router_config settings directly: {settings}. The router reads \
     these only from complexity_router_config, so there they configure nothing and are forwarded \
     to the provider as unknown request params, which rejects the call. Move them under \
     complexity_router_config."
)]
pub struct ComplexityPlacementError {
    pub settings: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ComplexityConfigWriteError {
    #[error(
        "complexity_router_config must be an object; the router would drop this deployment at load \
         time, so the write is rejected instead."
    )]
    NotAnObject,
    #[error(
        "complexity_router_config is invalid at {location}: {source}. The router would drop this \
         deployment at load time, so the write is rejected instead."
    )]
    Invalid {
        location: String,
        #[source]
        source: ComplexityConfigError,
    },
    #[error(
        "complexity_router_config is invalid at {location}: {message}. The router would drop this \
         deployment at load time, so the write is rejected instead."
    )]
    InvalidField { location: String, message: String },
    #[error(
        "complexity_router_config cannot be read: {message}. The router would drop this deployment \
         at load time, so the write is rejected instead."
    )]
    Unreadable { message: String },
}
