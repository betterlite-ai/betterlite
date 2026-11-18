use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use super::{
    AppliedInputType, AppliedInputTypes, CategoryScores, ExactScore, ModerationCategories,
};

// the SDK model dumps its field names; a provider slash key fills its field through the alias and is not dumped again
const CATEGORY_FIELDS: [&str; 13] = [
    "harassment",
    "harassment_threatening",
    "hate",
    "hate_threatening",
    "illicit",
    "illicit_violent",
    "self_harm",
    "self_harm_instructions",
    "self_harm_intent",
    "sexual",
    "sexual_minors",
    "violence",
    "violence_graphic",
];

const ALIASES: [(&str, &str); 13] = [
    ("harassment", "harassment"),
    ("harassment_threatening", "harassment/threatening"),
    ("hate", "hate"),
    ("hate_threatening", "hate/threatening"),
    ("illicit", "illicit"),
    ("illicit_violent", "illicit/violent"),
    ("self_harm", "self-harm"),
    ("self_harm_instructions", "self-harm/instructions"),
    ("self_harm_intent", "self-harm/intent"),
    ("sexual", "sexual"),
    ("sexual_minors", "sexual/minors"),
    ("violence", "violence"),
    ("violence_graphic", "violence/graphic"),
];

// the provider's keys that name no field, kept as the SDK model's extras; an underscore alias is no field name to it
fn extras(raw: &IndexMap<String, Value>) -> IndexMap<String, Value> {
    raw.iter()
        .filter(|(key, _)| !ALIASES.iter().any(|(_, alias)| alias == key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn field<'raw>(raw: &'raw IndexMap<String, Value>, name: &str) -> Option<&'raw Value> {
    let alias = ALIASES
        .iter()
        .find_map(|(field, alias)| (*field == name).then_some(*alias))
        .unwrap_or(name);
    raw.get(alias)
}

fn boolean(raw: &IndexMap<String, Value>, name: &str) -> Result<bool, String> {
    match field(raw, name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("the {name} category is not a boolean")),
        None => Err(format!("the {name} category is missing")),
    }
}

fn optional_boolean(raw: &IndexMap<String, Value>, name: &str) -> Result<Option<bool>, String> {
    match field(raw, name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("the {name} category is not a boolean")),
    }
}

fn applied(raw: &IndexMap<String, Value>, name: &str) -> Result<Vec<AppliedInputType>, String> {
    let Some(value) = field(raw, name) else {
        return Err(format!("the {name} applied input types are missing"));
    };
    let Value::Array(items) = value else {
        return Err(format!("the {name} applied input types are not a list"));
    };
    items
        .iter()
        .map(|item| {
            serde_json::from_value(item.clone())
                .map_err(|_error| format!("the {name} applied input type is not text or image"))
        })
        .collect()
}

fn score(raw: &IndexMap<String, Value>, name: &str) -> Result<ExactScore, String> {
    let Some(value) = field(raw, name) else {
        return Err(format!("the {name} category score is missing"));
    };
    let Value::Number(number) = value else {
        return Err(format!("the {name} category score is not a number"));
    };
    parse_decimal_lexeme(&number.to_string())
        .map(ExactScore)
        .map_err(|error| format!("the {name} category score is not an exact decimal: {error}"))
}

impl<'de> Deserialize<'de> for ModerationCategories {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = IndexMap::<String, Value>::deserialize(deserializer)?;
        let categories = ModerationCategories {
            harassment: boolean(&raw, "harassment").map_err(D::Error::custom)?,
            harassment_threatening: boolean(&raw, "harassment_threatening")
                .map_err(D::Error::custom)?,
            hate: boolean(&raw, "hate").map_err(D::Error::custom)?,
            hate_threatening: boolean(&raw, "hate_threatening").map_err(D::Error::custom)?,
            illicit: optional_boolean(&raw, "illicit").map_err(D::Error::custom)?,
            illicit_violent: optional_boolean(&raw, "illicit_violent").map_err(D::Error::custom)?,
            self_harm: boolean(&raw, "self_harm").map_err(D::Error::custom)?,
            self_harm_instructions: boolean(&raw, "self_harm_instructions")
                .map_err(D::Error::custom)?,
            self_harm_intent: boolean(&raw, "self_harm_intent").map_err(D::Error::custom)?,
            sexual: boolean(&raw, "sexual").map_err(D::Error::custom)?,
            sexual_minors: boolean(&raw, "sexual_minors").map_err(D::Error::custom)?,
            violence: boolean(&raw, "violence").map_err(D::Error::custom)?,
            violence_graphic: boolean(&raw, "violence_graphic").map_err(D::Error::custom)?,
            extra: extras(&raw),
        };
        Ok(categories)
    }
}

impl Serialize for ModerationCategories {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(CATEGORY_FIELDS.len() + self.extra.len()))?;
        map.serialize_entry("harassment", &self.harassment)?;
        map.serialize_entry("harassment_threatening", &self.harassment_threatening)?;
        map.serialize_entry("hate", &self.hate)?;
        map.serialize_entry("hate_threatening", &self.hate_threatening)?;
        map.serialize_entry("illicit", &self.illicit)?;
        map.serialize_entry("illicit_violent", &self.illicit_violent)?;
        map.serialize_entry("self_harm", &self.self_harm)?;
        map.serialize_entry("self_harm_instructions", &self.self_harm_instructions)?;
        map.serialize_entry("self_harm_intent", &self.self_harm_intent)?;
        map.serialize_entry("sexual", &self.sexual)?;
        map.serialize_entry("sexual_minors", &self.sexual_minors)?;
        map.serialize_entry("violence", &self.violence)?;
        map.serialize_entry("violence_graphic", &self.violence_graphic)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for AppliedInputTypes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = IndexMap::<String, Value>::deserialize(deserializer)?;
        Ok(AppliedInputTypes {
            harassment: applied(&raw, "harassment").map_err(D::Error::custom)?,
            harassment_threatening: applied(&raw, "harassment_threatening")
                .map_err(D::Error::custom)?,
            hate: applied(&raw, "hate").map_err(D::Error::custom)?,
            hate_threatening: applied(&raw, "hate_threatening").map_err(D::Error::custom)?,
            illicit: applied(&raw, "illicit").map_err(D::Error::custom)?,
            illicit_violent: applied(&raw, "illicit_violent").map_err(D::Error::custom)?,
            self_harm: applied(&raw, "self_harm").map_err(D::Error::custom)?,
            self_harm_instructions: applied(&raw, "self_harm_instructions")
                .map_err(D::Error::custom)?,
            self_harm_intent: applied(&raw, "self_harm_intent").map_err(D::Error::custom)?,
            sexual: applied(&raw, "sexual").map_err(D::Error::custom)?,
            sexual_minors: applied(&raw, "sexual_minors").map_err(D::Error::custom)?,
            violence: applied(&raw, "violence").map_err(D::Error::custom)?,
            violence_graphic: applied(&raw, "violence_graphic").map_err(D::Error::custom)?,
            extra: extras(&raw),
        })
    }
}

impl Serialize for AppliedInputTypes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(CATEGORY_FIELDS.len() + self.extra.len()))?;
        map.serialize_entry("harassment", &self.harassment)?;
        map.serialize_entry("harassment_threatening", &self.harassment_threatening)?;
        map.serialize_entry("hate", &self.hate)?;
        map.serialize_entry("hate_threatening", &self.hate_threatening)?;
        map.serialize_entry("illicit", &self.illicit)?;
        map.serialize_entry("illicit_violent", &self.illicit_violent)?;
        map.serialize_entry("self_harm", &self.self_harm)?;
        map.serialize_entry("self_harm_instructions", &self.self_harm_instructions)?;
        map.serialize_entry("self_harm_intent", &self.self_harm_intent)?;
        map.serialize_entry("sexual", &self.sexual)?;
        map.serialize_entry("sexual_minors", &self.sexual_minors)?;
        map.serialize_entry("violence", &self.violence)?;
        map.serialize_entry("violence_graphic", &self.violence_graphic)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for CategoryScores {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = IndexMap::<String, Value>::deserialize(deserializer)?;
        Ok(CategoryScores {
            harassment: score(&raw, "harassment").map_err(D::Error::custom)?,
            harassment_threatening: score(&raw, "harassment_threatening")
                .map_err(D::Error::custom)?,
            hate: score(&raw, "hate").map_err(D::Error::custom)?,
            hate_threatening: score(&raw, "hate_threatening").map_err(D::Error::custom)?,
            illicit: score(&raw, "illicit").map_err(D::Error::custom)?,
            illicit_violent: score(&raw, "illicit_violent").map_err(D::Error::custom)?,
            self_harm: score(&raw, "self_harm").map_err(D::Error::custom)?,
            self_harm_instructions: score(&raw, "self_harm_instructions")
                .map_err(D::Error::custom)?,
            self_harm_intent: score(&raw, "self_harm_intent").map_err(D::Error::custom)?,
            sexual: score(&raw, "sexual").map_err(D::Error::custom)?,
            sexual_minors: score(&raw, "sexual_minors").map_err(D::Error::custom)?,
            violence: score(&raw, "violence").map_err(D::Error::custom)?,
            violence_graphic: score(&raw, "violence_graphic").map_err(D::Error::custom)?,
            extra: extras(&raw),
        })
    }
}

impl Serialize for CategoryScores {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(CATEGORY_FIELDS.len() + self.extra.len()))?;
        map.serialize_entry("harassment", &self.harassment)?;
        map.serialize_entry("harassment_threatening", &self.harassment_threatening)?;
        map.serialize_entry("hate", &self.hate)?;
        map.serialize_entry("hate_threatening", &self.hate_threatening)?;
        map.serialize_entry("illicit", &self.illicit)?;
        map.serialize_entry("illicit_violent", &self.illicit_violent)?;
        map.serialize_entry("self_harm", &self.self_harm)?;
        map.serialize_entry("self_harm_instructions", &self.self_harm_instructions)?;
        map.serialize_entry("self_harm_intent", &self.self_harm_intent)?;
        map.serialize_entry("sexual", &self.sexual)?;
        map.serialize_entry("sexual_minors", &self.sexual_minors)?;
        map.serialize_entry("violence", &self.violence)?;
        map.serialize_entry("violence_graphic", &self.violence_graphic)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}
