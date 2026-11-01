use crate::facts::ProviderErrorBody;
use crate::facts::presence::Field;

// openai.APIError.__init__ reads type/param/code straight off this flat dict, unlike Azure's nested shape
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenaiErrorBody {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub param: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<OpenaiErrorDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenaiErrorDetail {
    #[serde(default)]
    pub message: Option<String>,
}

pub(super) fn typed_body(body: Option<&OpenaiErrorBody>) -> Option<ProviderErrorBody> {
    let fields = body?;
    let nested_message = fields
        .error
        .as_ref()
        .and_then(|error| error.message.clone());
    Some(ProviderErrorBody {
        message: fields.message.clone().map_or(Field::Missing, Field::Value),
        error: nested_message.map_or(Field::Missing, |message| {
            Field::Value(Box::new(ProviderErrorBody {
                message: Field::Value(message),
                error: Field::Missing,
                code: Field::Missing,
            }))
        }),
        code: Field::Missing,
    })
}
