use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

pub mod record;
use record::{HandleKind, RequestRecord};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequestRecordError {
    #[error("{bucket} metadata cannot test membership for a parent span")]
    MetadataMembership { bucket: String },
    #[error("{bucket} metadata cannot remove a parent span")]
    MetadataMutation { bucket: String },
    #[error("{bucket} metadata cannot remove a parent span by index")]
    MetadataRemoval { bucket: String },
}

pub struct RequestRecordProjection;

impl RequestRecordProjection {
    fn sanitize_bucket(
        fields: &mut BTreeMap<String, RequestRecord>,
        name: &str,
    ) -> Result<Option<RequestRecord>, RequestRecordError> {
        let Some(value) = fields.get_mut(name) else {
            return Ok(None);
        };
        let Some(bucket) = value.object_mut() else {
            return Self::sanitize_non_object(value, name);
        };
        let Some(span) = bucket.remove("litellm_parent_otel_span") else {
            return Ok(None);
        };
        bucket.insert(
            "litellm_parent_otel_span".to_owned(),
            RequestRecord::String("placeholder".to_owned()),
        );
        Ok(Some(span))
    }

    fn sanitize_non_object(
        value: &RequestRecord,
        name: &str,
    ) -> Result<Option<RequestRecord>, RequestRecordError> {
        match value {
            RequestRecord::String(text) if text.contains("litellm_parent_otel_span") => {
                Err(RequestRecordError::MetadataMutation {
                    bucket: name.to_owned(),
                })
            }
            RequestRecord::Array(values)
                if values.iter().any(|candidate| {
                    matches!(candidate, RequestRecord::String(text) if text == "litellm_parent_otel_span")
                }) =>
            {
                Err(RequestRecordError::MetadataRemoval {
                    bucket: name.to_owned(),
                })
            }
            RequestRecord::Null
            | RequestRecord::Bool(_)
            | RequestRecord::Signed(_)
            | RequestRecord::Unsigned(_)
            | RequestRecord::Decimal(_)
            | RequestRecord::Handle(_) => Err(RequestRecordError::MetadataMembership {
                bucket: name.to_owned(),
            }),
            RequestRecord::Array(_) | RequestRecord::String(_) | RequestRecord::Object(_) => {
                Ok(None)
            }
        }
    }

    fn restore(fields: &mut BTreeMap<String, RequestRecord>, name: &str, span: &RequestRecord) {
        let Some(bucket) = fields.get_mut(name).and_then(RequestRecord::object_mut) else {
            return;
        };
        if bucket.contains_key("litellm_parent_otel_span") {
            bucket.insert("litellm_parent_otel_span".to_owned(), span.clone());
        }
    }

    fn restore_all(record: &mut RequestRecord, span: &RequestRecord) {
        if matches!(span, RequestRecord::Null) {
            return;
        }
        let Some(fields) = record.object_mut() else {
            return;
        };
        Self::restore(fields, "metadata", span);
        Self::restore(fields, "litellm_metadata", span);
    }

    pub fn safe_copy(
        record: &mut RequestRecord,
        safe_memory_mode: bool,
    ) -> Result<Cow<'_, RequestRecord>, RequestRecordError> {
        if safe_memory_mode {
            return Ok(Cow::Borrowed(record));
        }
        let saved_metadata = record
            .object_mut()
            .map(|fields| Self::sanitize_bucket(fields, "metadata"))
            .transpose()?
            .flatten();
        let saved_internal = record
            .object_mut()
            .map(|fields| Self::sanitize_bucket(fields, "litellm_metadata"))
            .transpose()?
            .flatten();
        let copied = record.clone();
        if let Some(span) = saved_internal.or(saved_metadata) {
            Self::restore_all(record, &span);
        }
        Ok(Cow::Owned(copied))
    }

    #[must_use]
    pub fn independent_snapshot(record: &RequestRecord) -> RequestRecord {
        let Some(fields) = record.object() else {
            return record.clone();
        };
        let mut snapshot = BTreeMap::new();
        for (key, value) in fields {
            let copied = Self::snapshot_value(key, value);
            snapshot.insert(key.clone(), copied);
        }
        RequestRecord::Object(snapshot)
    }

    fn snapshot_value(key: &str, value: &RequestRecord) -> RequestRecord {
        if matches!(key, "metadata" | "litellm_metadata") {
            let Some(bucket) = value.object() else {
                return value.clone();
            };
            let mut copied = bucket.clone();
            let Some(original_span) = bucket.get("litellm_parent_otel_span") else {
                return RequestRecord::Object(copied);
            };
            copied.insert(
                "litellm_parent_otel_span".to_owned(),
                RequestRecord::String("placeholder".to_owned()),
            );
            copied.insert("litellm_parent_otel_span".to_owned(), original_span.clone());
            return RequestRecord::Object(copied);
        }
        value.clone()
    }

    #[must_use]
    pub fn filter_parameters(record: RequestRecord, max_depth: i32) -> RequestRecord {
        if max_depth <= 0_i32 {
            return record;
        }
        Self::filter_value(record, max_depth)
    }

    fn filter_value(record: RequestRecord, depth: i32) -> RequestRecord {
        if let RequestRecord::Handle(handle) = &record {
            if matches!(handle.kind, HandleKind::Exception | HandleKind::Callable) {
                return RequestRecord::Null;
            }
            if depth <= 0_i32 {
                return record;
            }
            if matches!(
                handle.kind,
                HandleKind::Logging | HandleKind::LiteLLMLoggingObj | HandleKind::Router
            ) {
                return RequestRecord::Null;
            }
        }
        if depth <= 0_i32 {
            return record;
        }
        match record {
            RequestRecord::Object(fields) => Self::filter_object(fields, depth),
            RequestRecord::Array(values) => Self::filter_array(values, depth),
            RequestRecord::Null
            | RequestRecord::Bool(_)
            | RequestRecord::Signed(_)
            | RequestRecord::Unsigned(_)
            | RequestRecord::Decimal(_)
            | RequestRecord::String(_)
            | RequestRecord::Handle(_) => record,
        }
    }

    fn filter_object(fields: BTreeMap<String, RequestRecord>, depth: i32) -> RequestRecord {
        let filtered = fields
            .into_iter()
            .filter_map(|(key, value)| Self::filter_item(value, depth).map(|item| (key, item)))
            .collect();
        RequestRecord::Object(filtered)
    }

    fn filter_array(values: Vec<RequestRecord>, depth: i32) -> RequestRecord {
        let filtered = values
            .into_iter()
            .filter_map(|value| Self::filter_item(value, depth))
            .collect();
        RequestRecord::Array(filtered)
    }

    fn filter_item(value: RequestRecord, depth: i32) -> Option<RequestRecord> {
        let filtered = Self::filter_value(value, depth.saturating_sub(1));
        (!matches!(filtered, RequestRecord::Null)).then_some(filtered)
    }

    #[must_use]
    pub fn filter_internal(record: RequestRecord, additional: &BTreeSet<String>) -> RequestRecord {
        let RequestRecord::Object(fields) = record else {
            return record;
        };
        let internal = [
            "skip_mcp_handler",
            "mcp_handler_context",
            "_skip_mcp_handler",
        ];
        RequestRecord::Object(
            fields
                .into_iter()
                .filter(|(key, _)| !internal.contains(&key.as_str()) && !additional.contains(key))
                .collect(),
        )
    }
}

pub struct GatewayCacheAttribution;
impl GatewayCacheAttribution {
    pub fn record(record: &mut RequestRecord, added: i32, every_deployment: bool) {
        if added <= 0_i32 {
            return;
        }
        let Some(fields) = record.object_mut() else {
            return;
        };
        let selected_bucket = ["litellm_metadata", "metadata"]
            .into_iter()
            .find(|name| fields.get(*name).and_then(RequestRecord::object).is_some());
        let Some(bucket_name) = selected_bucket else {
            return;
        };
        let marker = fields
            .get("model_info")
            .and_then(RequestRecord::object)
            .and_then(|info| info.get("id"))
            .cloned()
            .unwrap_or(RequestRecord::String(String::new()));
        let Some(bucket) = fields
            .get_mut(bucket_name)
            .and_then(RequestRecord::object_mut)
        else {
            return;
        };
        if bucket.get("litellm_gateway_injected_cache")
            == Some(&RequestRecord::String(String::new()))
        {
            return;
        }
        bucket.insert(
            "litellm_gateway_injected_cache".to_owned(),
            if every_deployment {
                RequestRecord::String(String::new())
            } else {
                marker
            },
        );
    }
}

#[path = "request_records/tests.rs"]
mod request_records_cases;
