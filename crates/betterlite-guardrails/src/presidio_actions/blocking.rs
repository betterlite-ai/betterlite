use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;

use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_analysis::wire::AnalyzeResponseItem;

pub fn raise_if_blocked_entities(
    analyze_results: &[AnalyzeResponseItem],
    pii_entities_config: &BTreeMap<String, PiiAction>,
    guardrail_name: &str,
) -> Result<(), PresidioActionsError> {
    for result in analyze_results {
        let Some(entity_type) = result.entity_type.as_deref() else {
            continue;
        };
        if pii_entities_config.get(entity_type) == Some(&PiiAction::Block) {
            return Err(PresidioActionsError::BlockedEntity {
                entity_type: entity_type.to_owned(),
                guardrail_name: guardrail_name.to_owned(),
            });
        }
    }
    Ok(())
}
