use betterlite_openai::api::file_type_records::{
    FileType, extension_from_mime, gemini_accepts, mime_from_extension,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: String,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    mimes: Vec<String>,
    #[serde(default)]
    file_types: Vec<String>,
}

fn parse_file_type(name: &str) -> Result<FileType, String> {
    let file_type = match name {
        "AAC" => FileType::Aac, "CSV" => FileType::Csv, "DOC" => FileType::Doc,
        "DOCX" => FileType::Docx, "FLAC" => FileType::Flac, "FLV" => FileType::Flv,
        "GIF" => FileType::Gif, "JPEG" => FileType::Jpeg, "JSON" => FileType::Json,
        "M4A" => FileType::M4a, "MPA" => FileType::Mpa, "MP3" => FileType::Mp3,
        "MPEG" => FileType::Mpeg, "MPGA" => FileType::Mpga, "PNG" => FileType::Png,
        "PDF" => FileType::Pdf, "TXT" => FileType::Txt, "3GPP" => FileType::ThreeGpp,
        "WEBP" => FileType::Webp, "WEBM" => FileType::Webm,
        other => return Err(format!("unknown file type: {other}")),
    };
    Ok(file_type)
}

fn mime_table(input: &Input) -> Result<Value, String> {
    let mut rows = Vec::new();
    for extension in &input.extensions {
        let mime = mime_from_extension(extension).map_err(|error| error.to_string())?;
        rows.push(json!({"extension": extension, "mime": mime}));
    }
    Ok(json!(rows))
}

fn lookup_edges(input: &Input) -> Result<Value, String> {
    let mut extensions = Vec::new();
    for extension in &input.extensions {
        extensions.push(mime_from_extension(extension).map_err(|error| error.to_string())?.to_owned());
    }
    let mut mimes = Vec::new();
    for mime in &input.mimes {
        mimes.push(extension_from_mime(mime).map_err(|error| error.to_string())?.to_owned());
    }
    Ok(json!({"extensions": extensions, "mimes": mimes}))
}

fn gemini_acceptance(input: &Input) -> Result<Value, String> {
    let mut accepted = Vec::new();
    for name in &input.file_types {
        accepted.push(gemini_accepts(parse_file_type(name)?));
    }
    Ok(json!(accepted))
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
