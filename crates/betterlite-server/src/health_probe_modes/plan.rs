use super::mode::HealthProbeMode;
use super::params::HealthProbeParams;
// the prompt and input a health endpoint passes to a probe; both are optional
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthProbeArguments {
    pub prompt: Option<String>,
    pub input: Option<Vec<String>>,
}
// what one health check will run, recorded before any call argument is built
pub struct HealthProbePlan {
    pub(crate) mode: HealthProbeMode,
    pub(crate) model: String,
    pub(crate) custom_llm_provider: String,
    pub(crate) params: HealthProbeParams,
    pub(crate) prompt: Option<String>,
    pub(crate) input: Option<Vec<String>>,
}
#[must_use]
pub fn plan_health_probe(
    mode: &str,
    model: &str,
    custom_llm_provider: &str,
    params: HealthProbeParams,
    arguments: HealthProbeArguments,
) -> Option<HealthProbePlan> {
    let selected = HealthProbeMode::try_from(mode).ok()?;
    let plan = HealthProbePlan {
        mode: selected,
        model: model.to_owned(),
        custom_llm_provider: custom_llm_provider.to_owned(),
        params,
        prompt: arguments.prompt,
        input: arguments.input,
    };
    Some(plan)
}
