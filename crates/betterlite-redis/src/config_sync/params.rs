#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncAppliedParam {
    GeneralSettings,
    RouterSettings,
    LitellmSettings,
    ModelCostMapReloadConfig,
    AnthropicBetaHeaders,
}

impl ResyncAppliedParam {
    pub const ALL: [Self; 5] = [
        Self::GeneralSettings,
        Self::RouterSettings,
        Self::LitellmSettings,
        Self::ModelCostMapReloadConfig,
        Self::AnthropicBetaHeaders,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::GeneralSettings => "general_settings",
            Self::RouterSettings => "router_settings",
            Self::LitellmSettings => "litellm_settings",
            Self::ModelCostMapReloadConfig => "model_cost_map_reload_config",
            Self::AnthropicBetaHeaders => "anthropic_beta_headers_reload_config",
        }
    }
}
