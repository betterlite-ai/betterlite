use indexmap::IndexMap;
use serde_json::Number;

use betterlite_prompts::arguments::json::{Json, Object};

// A class attribute as get_config sees it: a plain value, or one of the callables it skips
#[derive(Clone, Debug, PartialEq)]
pub enum ClassAttribute {
    Value(Json),
    Function,
    BuiltinFunction,
    ClassMethod,
    StaticMethod,
}

pub type ClassAttributes = IndexMap<String, ClassAttribute>;

// AmazonTitanG1Config.get_config and AmazonTitanV2Config.get_config: own-class attributes without dunder names, callables or None
#[must_use]
pub fn get_config(attributes: &ClassAttributes) -> Object {
    let mut config = Object::new();
    for (name, attribute) in attributes {
        let ClassAttribute::Value(value) = attribute else {
            continue;
        };
        if name.starts_with("__") || matches!(value, Json::Null) {
            continue;
        }
        config.insert(name.clone(), value.clone());
    }
    config
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TitanV2Settings {
    pub normalize: Option<bool>,
    pub dimensions: Option<i64>,
}

impl TitanV2Settings {
    // AmazonTitanV2Config.__init__ writes only its non-None arguments to the class, so None leaves a prior setting alone
    pub fn apply(&mut self, normalize: Option<bool>, dimensions: Option<i64>) {
        if let Some(flag) = normalize {
            self.normalize = Some(flag);
        }
        if let Some(length) = dimensions {
            self.dimensions = Some(length);
        }
    }

    // the class attributes get_config inspects, with an unset setting still there as None
    #[must_use]
    pub fn attributes(&self) -> ClassAttributes {
        let attributes = [
            ("normalize", class_value(self.normalize.map(Json::Bool))),
            (
                "dimensions",
                class_value(
                    self.dimensions
                        .map(|length| Json::Number(Number::from(length))),
                ),
            ),
        ];
        attributes
            .into_iter()
            .map(|(name, attribute)| (String::from(name), attribute))
            .collect()
    }
}

fn class_value(value: Option<Json>) -> ClassAttribute {
    ClassAttribute::Value(value.unwrap_or(Json::Null))
}
