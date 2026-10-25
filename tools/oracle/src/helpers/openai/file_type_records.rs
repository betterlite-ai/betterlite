use betterlite_openai::api::file_type_records::{
    FileType, extension_from_mime, mime_from_extension,
    gemini_accepts,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    function: String,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    mimes: Vec<String>,
    #[serde(default)]
    file_types: Vec<String>,
}

fn parse_file_type(name: &str) -> Result<FileType, String> {
    let value = Value::String(name.to_owned());
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn mime_table(input: &Input) -> Result<Value, String> {
            let mut rows = Vec::new();
            for extension in &input.extensions {
                let mime = mime_from_extension(extension)
                    .map_err(|error| error.to_string())?;
                rows.push(json!({"extension": extension, "mime": mime}));
            }
            Ok(json!(rows))
}

fn lookup_edges(input: &Input) -> Result<Value, String> {
            let extensions = input
                .extensions
                .iter()
                .map(|extension| {
                    mime_from_extension(extension)
                        .map(str::to_owned)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mimes = input
                .mimes
                .iter()
                .map(|mime| {
                    extension_from_mime(mime)
                        .map(str::to_owned)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(json!({"extensions": extensions, "mimes": mimes}))
}

fn gemini_acceptance(input: &Input) -> Result<Value, String> {
    input
            .file_types
            .iter()
            .map(|name| parse_file_type(name).map(gemini_accepts))
            .collect::<Result<Vec<_>, _>>()
            .map(|accepted| json!(accepted))
}

fn run_conversion(input: &Input) -> Result<Value, String> {
    match input.function.as_str() {
        "mime_extension_table" => mime_table(input),
        "mime_lookup_edges" => lookup_edges(input),
        "gemini_file_acceptance" => gemini_acceptance(input),
        other => Err(format!("unknown file_type_records function: {other}")),
    }
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run_conversion(input).map_err(|message| {
            Box::new(crate::helpers::HelperError {
                class: String::from("InvalidInput"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        })
    }
}
