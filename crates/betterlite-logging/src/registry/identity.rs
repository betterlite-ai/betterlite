use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Attribute {
    Flag(bool),
    Integer(i64),
    Text(String),
}

impl Attribute {
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Flag(set) => {
                if *set {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                }
            }
            Self::Integer(number) => number.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackIdentity {
    pub class_name: String,
    pub attributes: IndexMap<String, Attribute>,
}

impl CallbackIdentity {
    // LiteLLM's _get_custom_logger_key: the class name plus every public str, bool and int attribute
    #[must_use]
    pub fn key(&self) -> String {
        let mut key = self.class_name.clone();
        for (name, attribute) in &self.attributes {
            key.push('-');
            key.push_str(name);
            key.push('=');
            key.push_str(&attribute.text());
        }
        key
    }
}
