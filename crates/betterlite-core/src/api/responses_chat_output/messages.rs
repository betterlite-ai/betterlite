use std::collections::BTreeMap;

use betterlite_openai::api::responses_shapes::{GenericResponseOutputItem, OutputText};

use crate::api::responses_output_details::{citation_fields, image_results};

use super::output::OutputItem;
use super::records::ChatChoice;
use super::status::finish_status;

pub fn message_items<F>(choices: &[ChatChoice], uuid: &mut F) -> Vec<OutputItem>
where
    F: FnMut() -> String,
{
    let mut items = Vec::new();
    for choice in choices {
        let message = &choice.message;
        if !message.images.is_empty() {
            let images = image_results(
                Some(&message.images),
                choice.finish_reason.as_deref(),
                &mut *uuid,
            );
            items.extend(images.into_iter().map(OutputItem::Image));
            continue;
        }
        let Some(content) = message.content.as_deref() else {
            continue;
        };
        items.push(OutputItem::Message(GenericResponseOutputItem {
            r#type: "message".to_owned(),
            id: format!("msg_{}", uuid()),
            status: finish_status(choice.finish_reason.as_deref()).to_owned(),
            role: message.role.clone(),
            content: vec![OutputText {
                r#type: Some("output_text".to_owned()),
                text: Some(content.to_owned()),
                annotations: Some(citation_fields(Some(&message.annotations))),
                extra: BTreeMap::new(),
            }],
            phase: None,
            extra: BTreeMap::new(),
        }));
    }
    items
}
