use betterlite_config::sections::tier_definitions as tier_section;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::error::ComplexityConfigWriteError;
use super::values::encode;

// LiteLLM reads a list pool entry by entry; serde could read a short list as one detailed entry
pub(super) fn reject_swallowed_pools(
    fields: &JsonObject,
) -> Result<(), ComplexityConfigWriteError> {
    for key in ["tiers", "tier_models"] {
        let Some(Json::Object(pools)) = fields.get(key) else {
            continue;
        };
        for (tier, declared) in pools {
            let Json::List(written) = declared else {
                continue;
            };
            for (index, entry) in written.iter().enumerate() {
                let location = format!("{key}.{tier}.{index}");
                match entry {
                    Json::Text(_) | Json::Object(_) => decode_entry(entry, &location)?,
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                        return Err(ComplexityConfigWriteError::InvalidTierEntry { location });
                    }
                }
            }
        }
    }
    Ok(())
}

fn decode_entry(entry: &Json, location: &str) -> Result<(), ComplexityConfigWriteError> {
    let text = encode(entry)?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    match serde_path_to_error::deserialize::<_, tier_section::TierEntry>(&mut deserializer) {
        Ok(_) => Ok(()),
        Err(source) => Err(ComplexityConfigWriteError::InvalidField {
            location: location.to_owned(),
            source,
        }),
    }
}
