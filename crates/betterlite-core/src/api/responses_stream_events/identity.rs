use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;

use super::record::{Target, slot_target};
use crate::api::response_id_traversal::container::encode_container_id;
use crate::api::response_id_traversal::record::Record;
use crate::api::response_id_traversal::response_id::{ResponseRewriteError, transcode};

pub fn encode(
    event: &mut ResponsesAPIStreamingResponse,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), ResponseRewriteError> {
    let Some(target) = slot_target(event) else {
        return Ok(());
    };
    match target {
        Target::Extra(extra) => {
            let mut value: Json = transcode(extra)?;
            encode_value(&mut value, provider, model_id);
            *extra = transcode(&value)?;
            Ok(())
        }
        Target::Extras(extras) => {
            let mut value: Json = transcode(extras)?;
            encode_values(&mut value, provider, model_id);
            *extras = transcode(&value)?;
            Ok(())
        }
        Target::Value(value) => {
            encode_value(value, provider, model_id);
            Ok(())
        }
        Target::Values(values) => {
            encode_values(values, provider, model_id);
            Ok(())
        }
    }
}

fn encode_value(value: &mut Json, provider: Option<&str>, model_id: Option<&str>) {
    let mut record = Record::from_json(value.clone());
    encode_container_id(&mut record, provider, model_id);
    *value = record.into_json();
}

fn encode_values(value: &mut Json, provider: Option<&str>, model_id: Option<&str>) {
    let Json::List(items) = value else {
        return;
    };
    for item in items {
        encode_value(item, provider, model_id);
    }
}
