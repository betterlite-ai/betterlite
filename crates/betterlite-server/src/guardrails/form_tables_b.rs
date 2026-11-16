mod data;
mod inspection;
mod tools;
mod vendors;

mod tests;

use crate::guardrails::provider_forms::spec::FormModel;

#[must_use]
#[rustfmt::skip]
pub fn tables_b() -> Vec<(&'static str, FormModel)> {
    vec![
        ("agent_365", data::model(&inspection::AGENT_365)),
        ("deepkeep", data::model(&vendors::DEEPKEEP)),
        ("vigil_guard", data::model(&vendors::VIGIL_GUARD)),
        ("cato_networks", data::model(&inspection::CATO_NETWORKS)),
        ("custom_code", data::model(&tools::CUSTOM_CODE)),
        ("repelloai", data::model(&vendors::REPELLOAI)),
        ("aporia", data::model(&vendors::APORIA)),
        ("tool_permission", data::model(&tools::TOOL_PERMISSION)),
        ("ibm_guardrails", data::model(&inspection::IBM_GUARDRAILS)),
        ("pangea", data::model(&inspection::PANGEA)),
        ("prompt_security", data::model(&vendors::PROMPT_SECURITY)),
        ("enkryptai", data::model(&vendors::ENKRYPTAI)),
        ("straiker", data::model(&vendors::STRAIKER)),
        ("cisco_ai_defense", data::model(&inspection::CISCO_AI_DEFENSE)),
        ("aim", data::model(&inspection::AIM)),
        ("singulr", data::model(&inspection::SINGULR)),
        ("onyx", data::model(&vendors::ONYX)),
        ("akto", data::model(&inspection::AKTO)),
        ("pillar", data::model(&vendors::PILLAR)),
        ("lasso", data::model(&vendors::LASSO)),
        ("generic_guardrail_api", data::model(&tools::GENERIC_GUARDRAIL_API)),
        ("ovalix", data::model(&vendors::OVALIX)),
        ("compresr", data::model(&vendors::COMPRESR)),
        ("model_armor", data::model(&inspection::MODEL_ARMOR)),
    ]
}
