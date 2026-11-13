use betterlite_openai::chat::response::ChatChoice;
use serde_json::{Map, Value};

const GENERATED_FILES_KEY: &str = "_litellm_generated_files";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentBlock {
    pub kind: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub name: String,
    pub mime_type: String,
    pub content_base64: String,
    pub size: usize,
}

// Some response objects reject the attribute, and LiteLLM swallows the AttributeError they raise
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedFilesAttribute {
    Rejected,
    Writable(Vec<GeneratedFile>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectResponse {
    pub content: Vec<ContentBlock>,
    pub choices: Vec<ChatChoice>,
    pub generated_files: GeneratedFilesAttribute,
    // None is a response without a model_extra attribute, and LiteLLM's hasattr check then skips the write
    pub model_extra: Option<Map<String, Value>>,
}

// LiteLLM reads dict responses with map lookups and typed responses through their attributes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookResponse {
    Dict(Map<String, Value>),
    Object(ObjectResponse),
}

impl ObjectResponse {
    pub fn set_generated_files(&mut self, generated_files: &[GeneratedFile]) {
        if let GeneratedFilesAttribute::Writable(attribute) = &mut self.generated_files {
            *attribute = generated_files.to_vec();
        }
    }
}

#[must_use]
pub fn attach_files_to_response(
    response: HookResponse,
    generated_files: &[GeneratedFile],
) -> HookResponse {
    if generated_files.is_empty() {
        return response;
    }
    let files = generated_files_json(generated_files);
    match response {
        HookResponse::Dict(mut fields) => {
            fields.insert(GENERATED_FILES_KEY.to_owned(), files);
            HookResponse::Dict(fields)
        }
        HookResponse::Object(mut object) => {
            object.set_generated_files(generated_files);
            if let Some(extra) = object.model_extra.as_mut() {
                extra.insert(GENERATED_FILES_KEY.to_owned(), files);
            }
            HookResponse::Object(object)
        }
    }
}

fn generated_files_json(generated_files: &[GeneratedFile]) -> Value {
    Value::Array(
        generated_files
            .iter()
            .map(|file| {
                let mut fields = Map::new();
                fields.insert("name".to_owned(), Value::String(file.name.clone()));
                fields.insert(
                    "mime_type".to_owned(),
                    Value::String(file.mime_type.clone()),
                );
                fields.insert(
                    "content_base64".to_owned(),
                    Value::String(file.content_base64.clone()),
                );
                fields.insert("size".to_owned(), Value::from(file.size));
                Value::Object(fields)
            })
            .collect(),
    )
}
