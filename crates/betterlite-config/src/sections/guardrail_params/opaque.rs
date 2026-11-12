use std::collections::BTreeMap;
use std::fmt;

use serde::de::{Deserializer, Error as DeError, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::sections::guardrail_identity::item::GuardrailValue;

// serde_json hands a number to a visitor as a one-entry map under this key when arbitrary_precision is on
const NUMBER_KEY: &str = "$serde_json::private::Number";

#[derive(Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OpaqueParams(pub BTreeMap<String, GuardrailValue>);

impl fmt::Debug for OpaqueParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

pub(super) struct ExactValue(pub GuardrailValue);

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for OpaqueParams {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = BTreeMap::<String, ExactValue>::deserialize(deserializer)?;
        Ok(Self(
            entries
                .into_iter()
                .map(|(key, value)| (key, value.0))
                .collect(),
        ))
    }
}

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for ExactValue {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

// ast-grep-ignore: no-custom-serde
impl<'de> Visitor<'de> for ValueVisitor {
    type Value = ExactValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a guardrail parameter value")
    }

    fn visit_unit<E>(self) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(GuardrailValue::Null))
    }

    fn visit_none<E>(self) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(GuardrailValue::Null))
    }

    fn visit_bool<E>(self, v: bool) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(GuardrailValue::Boolean(v)))
    }

    fn visit_i64<E>(self, v: i64) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(GuardrailValue::SignedInteger(v)))
    }

    fn visit_u64<E>(self, v: u64) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(match i64::try_from(v) {
            Ok(signed) => GuardrailValue::SignedInteger(signed),
            Err(_overflow) => GuardrailValue::UnsignedInteger(v),
        }))
    }

    fn visit_f64<E>(self, v: f64) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(GuardrailValue::Decimal(v)))
    }

    fn visit_str<E>(self, v: &str) -> Result<ExactValue, E>
    where
        E: DeError,
    {
        Ok(ExactValue(GuardrailValue::Text(v.to_owned())))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<ExactValue, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<ExactValue>()? {
            items.push(item.0);
        }
        Ok(ExactValue(GuardrailValue::Array(items)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<ExactValue, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        let Some(first) = map.next_key::<String>()? else {
            return Ok(ExactValue(GuardrailValue::Object(entries)));
        };
        if first == NUMBER_KEY {
            let text = map.next_value::<String>()?;
            return number_from_text(&text).map(ExactValue);
        }
        entries.insert(first, map.next_value::<ExactValue>()?.0);
        while let Some(key) = map.next_key::<String>()? {
            entries.insert(key, map.next_value::<ExactValue>()?.0);
        }
        Ok(ExactValue(GuardrailValue::Object(entries)))
    }
}

fn number_from_text<E>(text: &str) -> Result<GuardrailValue, E>
where
    E: DeError,
{
    if let Ok(signed) = text.parse::<i64>() {
        return Ok(GuardrailValue::SignedInteger(signed));
    }
    if let Ok(unsigned) = text.parse::<u64>() {
        return Ok(GuardrailValue::UnsignedInteger(unsigned));
    }
    text.parse::<f64>()
        .map(GuardrailValue::Decimal)
        .map_err(|_unparsable| E::custom("invalid number"))
}
