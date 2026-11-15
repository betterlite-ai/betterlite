mod inspection;
mod tools;
mod vendors;

mod tests;

use crate::guardrails::provider_forms::spec::FormModel;

#[must_use]
#[rustfmt::skip]
pub fn tables_b() -> Vec<(&'static str, FormModel)> {
    vec![
        ("agent_365", inspection::agent_365_model()),
        ("deepkeep", vendors::deepkeep_model()),
        ("vigil_guard", vendors::vigil_guard_model()),
        ("cato_networks", inspection::cato_networks_model()),
        ("custom_code", tools::custom_code_model()),
        ("repelloai", vendors::repelloai_model()),
        ("aporia", vendors::aporia_model()),
        ("tool_permission", tools::tool_permission_model()),
        ("ibm_guardrails", inspection::ibm_guardrails_model()),
        ("pangea", inspection::pangea_model()),
        ("prompt_security", vendors::prompt_security_model()),
        ("enkryptai", vendors::enkryptai_model()),
        ("straiker", vendors::straiker_model()),
        ("cisco_ai_defense", inspection::cisco_ai_defense_model()),
        ("aim", inspection::aim_model()),
        ("singulr", inspection::singulr_model()),
        ("onyx", vendors::onyx_model()),
        ("akto", inspection::akto_model()),
        ("pillar", vendors::pillar_model()),
        ("lasso", vendors::lasso_model()),
        ("generic_guardrail_api", tools::generic_guardrail_api_model()),
        ("ovalix", vendors::ovalix_model()),
        ("compresr", vendors::compresr_model()),
        ("model_armor", inspection::model_armor_model()),
    ]
}
