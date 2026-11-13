use super::lax::LaxInteger;
use crate::json::{Json, Object};
use crate::ocr_config::decode::{OcrFieldKind, decode_value};
use crate::ocr_error::OcrError;
use crate::response::{OCRPage, OCRPageImage, OCRResponse, OCRUsageInfo};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CohereResponse {
    #[serde(default)]
    pages: Vec<CoherePage>,
    meta: Option<CohereMeta>,
}

#[derive(Debug, Deserialize)]
struct CoherePage {
    index: Option<LaxInteger>,
    markdown: Option<CohereMarkdown>,
    blocks: Option<Vec<Object>>,
}

#[derive(Debug, Deserialize)]
struct CohereMarkdown {
    #[serde(default)]
    content: String,
    images: Option<Vec<Object>>,
}

#[derive(Debug, Deserialize)]
struct CohereMeta {
    billed_units: Option<CohereBilledUnits>,
}

#[derive(Debug, Deserialize)]
struct CohereBilledUnits {
    pages: Option<LaxInteger>,
}

pub fn normalize_response(model: &str, response: CohereResponse) -> Result<OCRResponse, OcrError> {
    let billed = response
        .meta
        .as_ref()
        .and_then(|meta| meta.billed_units.as_ref())
        .and_then(|units| units.pages.as_ref())
        .map(|pages| pages.0);
    let pages_processed = match billed {
        Some(pages) => pages,
        None => i64::try_from(response.pages.len())
            .map_err(|_source| OcrError::NumericRange("pages"))?,
    };
    let pages = response
        .pages
        .into_iter()
        .enumerate()
        .map(|(position, page)| normalize_page(page, position))
        .collect::<Result<Vec<_>, OcrError>>()?;
    Ok(OCRResponse {
        pages,
        model: model.to_owned(),
        document_annotation: None,
        usage_info: Some(OCRUsageInfo {
            pages_processed: Some(pages_processed),
            pages_processed_annotation: None,
            credits: None,
            doc_size_bytes: None,
            extra: Object::new(),
        }),
        content: None,
        tables: None,
        key_value_pairs: None,
        object: "ocr".to_owned(),
        extra: Object::new(),
        provider_native_response: None,
    })
}

fn normalize_page(page: CoherePage, position: usize) -> Result<OCRPage, OcrError> {
    let index = match page.index {
        Some(index) => index.0,
        None => i64::try_from(position).map_err(|_source| OcrError::NumericRange("page index"))?,
    };
    let (markdown, images) = match page.markdown {
        Some(markdown) => {
            let images = markdown
                .images
                .filter(|images| !images.is_empty())
                .map(|images| {
                    images
                        .into_iter()
                        .enumerate()
                        .map(|(image_index, image)| page_image(image, position, image_index))
                        .collect::<Result<Vec<_>, OcrError>>()
                })
                .transpose()?;
            (markdown.content, images)
        }
        None => (String::new(), None),
    };
    let mut extra = Object::new();
    if let Some(blocks) = page.blocks {
        let _previous = extra.insert(
            "blocks".to_owned(),
            Json::List(blocks.into_iter().map(Json::Object).collect()),
        );
    }
    Ok(OCRPage {
        index,
        markdown,
        images,
        dimensions: None,
        extra,
    })
}

fn page_image(
    mut image: Object,
    position: usize,
    image_index: usize,
) -> Result<OCRPageImage, OcrError> {
    if let Some(Json::Object(bounding_box)) = image.get("bounding_box").cloned() {
        let _previous = image.insert("bbox".to_owned(), Json::Object(bounding_box));
    }
    decode_value(
        &Json::Object(image),
        &format!("pages[{position}].markdown.images[{image_index}]"),
        OcrFieldKind::Response,
    )
}
