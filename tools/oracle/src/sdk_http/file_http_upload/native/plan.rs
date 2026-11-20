use betterlite_core::api::file_http_upload::plan::{
    MediaPlan, MultipartPlan, PresignedPlan, TwoStepPlan, UploadPlan,
};
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::UploadData;

use crate::sdk_http::file_http_upload::{
    Content, MediaSpec, MultipartSpec, Plan, PresignedSpec, TwoStepSpec,
};

use super::PLACEHOLDER;

pub(super) fn upload_plan(spec: &Plan, origin: &str) -> UploadPlan {
    match spec {
        Plan::TwoStep(two_step) => UploadPlan::TwoStep(Box::new(two_step_plan(two_step, origin))),
        Plan::Presigned(presigned) => {
            UploadPlan::Presigned(Box::new(presigned_plan(presigned, origin)))
        }
        Plan::Media(media) => UploadPlan::Media(Box::new(media_plan(media))),
        Plan::Multipart(multipart) => UploadPlan::Multipart(Box::new(multipart_plan(multipart))),
        Plan::Text(text) => UploadPlan::Text(text.clone()),
        Plan::Raw(bytes) => UploadPlan::Bytes(bytes.clone()),
    }
}

fn two_step_plan(spec: &TwoStepSpec, origin: &str) -> TwoStepPlan {
    TwoStepPlan {
        initial_method: spec.initial_request.method.clone(),
        initial_url: resolved(&spec.initial_request.url, origin),
        initial_headers: spec.initial_request.headers.clone(),
        initial_data: Json::Object(spec.initial_request.data.clone()),
        upload_method: spec.upload_request.method.clone(),
        upload_url: resolved(&spec.upload_request.url, origin),
        upload_headers: spec.upload_request.headers.clone(),
        upload_data: upload_data(&spec.upload_request.data),
        url_location: spec.upload_url_location,
        url_key: spec.upload_url_key.clone(),
    }
}

fn presigned_plan(spec: &PresignedSpec, origin: &str) -> PresignedPlan {
    PresignedPlan {
        method: spec.method.clone(),
        url: resolve(&spec.url, origin),
        headers: spec.headers.clone(),
        data: upload_data(&spec.data),
    }
}

fn media_plan(spec: &MediaSpec) -> MediaPlan {
    MediaPlan {
        body_stream: Box::new(spec.chunks.clone().into_iter()),
        content_type: spec.content_type.clone(),
    }
}

fn multipart_plan(spec: &MultipartSpec) -> MultipartPlan {
    MultipartPlan {
        parts: spec
            .parts
            .iter()
            .map(|part| MultipartPart {
                name: part.name.clone(),
                file_name: part.filename.clone(),
                content_type: part.content_type.clone(),
                data: part.content.bytes(),
            })
            .collect(),
    }
}

fn upload_data(content: &Content) -> UploadData {
    match content {
        Content::Text(text) => UploadData::Text(text.clone()),
        Content::Raw(bytes) => UploadData::Bytes(bytes.clone()),
    }
}

fn resolved(url: &Option<String>, origin: &str) -> Option<String> {
    match url {
        Some(url) => Some(resolve(url, origin)),
        None => None,
    }
}

fn resolve(url: &str, origin: &str) -> String {
    url.replace(PLACEHOLDER, origin)
}
