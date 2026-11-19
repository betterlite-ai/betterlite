use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use crate::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use crate::s3_transport::naming::object_key;
use crate::s3_transport::options::PartitionGranularity;

const AUDIT_PREFIX: &str = "audit_logs/";
const UNKNOWN_ID: &str = "unknown";

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuditElementError {
    #[error("the S3 audit record is not a JSON object")]
    NotAnObject,
    #[error("the S3 audit record id is not a string")]
    IdNotText,
}

pub fn audit_element(
    s3_path: &str,
    granularity: PartitionGranularity,
    audit_log: &Json,
    now: DateTime<Utc>,
) -> Result<S3BatchLoggingElement<Json>, AuditElementError> {
    let Json::Object(record) = audit_log else {
        return Err(AuditElementError::NotAnObject);
    };
    let identifier = match record.get("id") {
        None | Some(Json::Null) => UNKNOWN_ID,
        Some(Json::Text(text)) => text.as_str(),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(AuditElementError::IdNotText);
        }
    };
    let file_name = format!("{}_{identifier}", now.format("%H-%M-%S"));
    Ok(S3BatchLoggingElement {
        payload: audit_log.clone(),
        s3_object_key: object_key(
            s3_path,
            AUDIT_PREFIX,
            now.naive_utc(),
            &file_name,
            granularity,
        ),
        s3_object_download_filename: format!("audit-{identifier}.json"),
        body: None,
        content_type: ContentType::Json,
        retrying_since: None,
    })
}
