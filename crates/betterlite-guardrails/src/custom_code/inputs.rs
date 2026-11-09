use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

// GenericGuardrailAPIInputs is an open TypedDict, so unnamed keys travel through `extra`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomCodeInputs {
    pub texts: Option<Vec<String>>,
    pub images: Option<Vec<String>>,
    pub tool_calls: Option<Vec<GuardrailValue>>,
    pub extra: BTreeMap<String, GuardrailValue>,
}

impl CustomCodeInputs {
    #[must_use]
    pub fn to_value(&self) -> GuardrailValue {
        let mut object = self.extra.clone();
        if let Some(texts) = &self.texts {
            let values = texts
                .iter()
                .map(|text| GuardrailValue::Text(text.clone()))
                .collect();
            object.insert("texts".to_owned(), GuardrailValue::Array(values));
        }
        if let Some(images) = &self.images {
            let values = images
                .iter()
                .map(|image| GuardrailValue::Text(image.clone()))
                .collect();
            object.insert("images".to_owned(), GuardrailValue::Array(values));
        }
        if let Some(tool_calls) = &self.tool_calls {
            object.insert(
                "tool_calls".to_owned(),
                GuardrailValue::Array(tool_calls.clone()),
            );
        }
        GuardrailValue::Object(object)
    }
}
