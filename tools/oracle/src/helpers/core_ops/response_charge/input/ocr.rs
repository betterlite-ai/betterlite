use betterlite_models::ocr_pricing::{OcrBilling, OcrResponseKind, OcrUsage};

use super::Input;

pub(crate) fn billing(input: &Input) -> Option<OcrBilling> {
    let usage = input.ocr.as_ref()?;
    Some(OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: Some(OcrUsage {
            credits: usage
                .credits
                .as_ref()
                .and_then(|number| number.to_string().parse().ok()),
            pages_processed: usage.pages_processed,
            pages_processed_annotation: usage.pages_processed_annotation.unwrap_or(0),
        }),
    })
}
