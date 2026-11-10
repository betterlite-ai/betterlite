use crate::helpers::{Helper, HelperResult};

mod common;
mod handlers;
mod input;
mod tests;
mod views;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            input::Input::FileUploadMultipart(case) => handlers::file_upload_multipart(case),
            input::Input::NativeFileObject(case) => handlers::native_file_object(case),
            input::Input::CreatedAtTimezone(case) => handlers::created_at_timezone(case),
            input::Input::FileListPagination(case) => handlers::file_list_pagination(case),
            input::Input::FileDeleteShape(case) => handlers::file_delete_shape(case),
            input::Input::FileContentBytes(case) => handlers::file_content_bytes(case),
            input::Input::FileBetaHeader(case) => handlers::file_beta_header(case),
        }
    }
}
