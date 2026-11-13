use std::ops::Mul as _;

use crate::json::Object;
use crate::ocr_error::OcrError;
use crate::response::{OCRPage, OCRPageDimensions, OCRResponse, OCRUsageInfo};

use super::wire::{AnalyzeOperation, AnalyzePage, KnownStatus, OperationStatus};
use super::{AZURE_DI_DEFAULT_HEIGHT, AZURE_DI_DEFAULT_WIDTH};

pub(super) fn normalize(
    model: &str,
    response: AnalyzeOperation,
    dpi: i64,
) -> Result<OCRResponse, OcrError> {
    let status = response.status;
    if status != Some(OperationStatus::Known(KnownStatus::Succeeded)) {
        let text = status.map_or_else(|| "None".to_owned(), |state| state.text().to_owned());
        return Err(OcrError::OperationStatus(text));
    }
    let result = response.analyze_result.unwrap_or_default();
    let pages = result
        .pages
        .into_iter()
        .map(|page| normalize_page(&page, dpi))
        .collect::<Result<Vec<_>, _>>()?;
    let pages_processed =
        i64::try_from(pages.len()).map_err(|_source| OcrError::NumericRange("pages"))?;
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
        content: result.content,
        tables: result.tables,
        key_value_pairs: result.key_value_pairs,
        object: "ocr".to_owned(),
        extra: Object::new(),
        provider_native_response: None,
    })
}

fn normalize_page(page: &AnalyzePage, dpi: i64) -> Result<OCRPage, OcrError> {
    // Azure page numbers are 1-based, so a lower one is rejected instead of keeping a negative index
    let index = page
        .page_number
        .map_or(1, |number| number.0)
        .checked_sub(1)
        .filter(|index| *index >= 0)
        .ok_or(OcrError::NumericRange("page.pageNumber"))?;
    let dimensions = convert_dimensions(
        page.width.map_or(AZURE_DI_DEFAULT_WIDTH, |number| number.0),
        page.height
            .map_or(AZURE_DI_DEFAULT_HEIGHT, |number| number.0),
        page.unit.as_deref().unwrap_or("inch"),
        dpi,
    )?;
    let markdown = page
        .lines
        .iter()
        .map(|line| line.content.as_deref().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(OCRPage {
        index,
        markdown,
        images: None,
        dimensions: Some(dimensions),
        extra: Object::new(),
    })
}

fn convert_dimensions(
    width: f64,
    height: f64,
    unit: &str,
    dpi: i64,
) -> Result<OCRPageDimensions, OcrError> {
    let scale = if unit == "inch" {
        // Rust has no lossless i64-to-f64 conversion and an as cast is forbidden
        serde_json::Number::from(dpi)
            .as_f64()
            .ok_or(OcrError::NumericRange("dpi"))?
    } else {
        1.0_f64
    };
    Ok(OCRPageDimensions {
        width: Some(pixel_dimension(width, scale, "page.width")?),
        height: Some(pixel_dimension(height, scale, "page.height")?),
        dpi: Some(dpi),
    })
}

fn pixel_dimension(value: f64, scale: f64, field: &'static str) -> Result<i64, OcrError> {
    // truncating toward zero keeps LiteLLM's cast result and an out-of-range value fails the parse
    format!("{:.0}", value.mul(scale).trunc())
        .parse::<i64>()
        .map_err(|_source| OcrError::NumericRange(field))
}
