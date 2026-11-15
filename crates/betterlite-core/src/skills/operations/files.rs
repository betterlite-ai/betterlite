use super::SkillUpload;
use super::SkillUploadType;

const DEFAULT_FILE_TYPE: &str = "application/zip";

pub(super) struct SelectedFile<'request> {
    pub(super) content: Option<&'request [u8]>,
    pub(super) name: Option<&'request str>,
    pub(super) file_type: Option<&'request str>,
}

// LiteLLM lets the first uploaded tuple replace explicit fields only when no bytes were given
pub(super) fn selected_file<'request>(
    files: Option<&'request [SkillUpload<'request>]>,
    file_content: Option<&'request [u8]>,
    file_name: Option<&'request str>,
    file_type: Option<&'request str>,
) -> SelectedFile<'request> {
    let mut selected = SelectedFile {
        content: file_content,
        name: file_name,
        file_type,
    };
    let bytes_missing = file_content.map_or(true, <[u8]>::is_empty);
    let Some(uploads) = files.filter(|uploads| !uploads.is_empty() && bytes_missing) else {
        return selected;
    };
    let Some(upload) = uploads.first() else {
        return selected;
    };
    selected.content = Some(upload.file_content);
    selected.name = Some(upload.file_name);
    selected.file_type = match upload.file_type {
        SkillUploadType::Defaulted => Some(DEFAULT_FILE_TYPE),
        SkillUploadType::Given(given) => given,
    };
    selected
}
