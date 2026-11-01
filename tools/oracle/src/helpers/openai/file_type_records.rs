use betterlite_openai::api::file_type_records::{
    FileType, extension_from_mime, gemini_accepts, mime_from_extension,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    MimeExtensionTable {
        extensions: Vec<String>,
    },
    MimeLookupEdges {
        extensions: Vec<String>,
        mimes: Vec<String>,
    },
    GeminiFileAcceptance {
        file_types: Vec<FileType>,
    },
}

fn mime_table(extensions: &[String]) -> Result<Value, String> {
    let mut rows = Vec::new();
    for extension in extensions {
        let mime = mime_from_extension(extension).map_err(|error| error.to_string())?;
        rows.push(json!({"extension": extension, "mime": mime}));
    }
    Ok(json!(rows))
}

fn lookup_edges(extensions: &[String], mimes: &[String]) -> Result<Value, String> {
    let mut mime_per_extension = Vec::new();
    for extension in extensions {
        mime_per_extension.push(
            mime_from_extension(extension)
                .map_err(|error| error.to_string())?
                .to_owned(),
        );
    }
    let mut extension_per_mime = Vec::new();
    for mime in mimes {
        extension_per_mime.push(
            extension_from_mime(mime)
                .map_err(|error| error.to_string())?
                .to_owned(),
        );
    }
    Ok(json!({"extensions": mime_per_extension, "mimes": extension_per_mime}))
}

fn run_conversion(input: &Input) -> Result<Value, String> {
    match input {
        Input::MimeExtensionTable { extensions } => mime_table(extensions),
        Input::MimeLookupEdges { extensions, mimes } => lookup_edges(extensions, mimes),
        Input::GeminiFileAcceptance { file_types } => {
            let accepted: Vec<_> = file_types.iter().copied().map(gemini_accepts).collect();
            Ok(json!(accepted))
        }
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
