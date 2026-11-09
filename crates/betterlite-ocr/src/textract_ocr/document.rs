use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::ocr_config::{OcrProviderConfig, OcrRequestContext, OcrTransform};
use crate::ocr_document::{InlineDocument, OcrDocument, inline_remote_document};
use crate::ocr_error::OcrError;
use crate::textract_ocr::wire::TextractDocument;

pub(crate) const SYNC_DOCUMENT_MAX_BYTES: usize = 10 * 1024 * 1024;

const HEALTH_CHECK_IMAGE_DATA_URI: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC";

pub(crate) fn health_check_document() -> OcrDocument {
    OcrDocument::ImageUrl {
        image_url: HEALTH_CHECK_IMAGE_DATA_URI.to_owned(),
        extra: BTreeMap::new(),
    }
}

pub(crate) fn document_bytes(document: &OcrDocument) -> Result<TextractDocument, OcrError> {
    let inline = InlineDocument::parse(document.source())?.ok_or(OcrError::InvalidDataUri)?;
    Ok(TextractDocument {
        bytes: STANDARD.encode(inline.decode(SYNC_DOCUMENT_MAX_BYTES)?),
    })
}

pub(crate) async fn inlined_request<C>(
    config: &C,
    document: OcrDocument,
    transform: OcrTransform<'_, C::OcrParams>,
    context: OcrRequestContext<'_>,
) -> Result<C::ProviderRequest, OcrError>
where
    C: OcrProviderConfig,
{
    config.transform_ocr_request(
        inline_remote_document(context.client, document, context.connection).await?,
        transform,
    )
}
