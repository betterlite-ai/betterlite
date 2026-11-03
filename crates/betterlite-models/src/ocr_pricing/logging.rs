use super::single_log_line;

pub(super) fn warn_missing_published_pricing(model: &str, custom_llm_provider: Option<&str>) {
    tracing::warn!(
        model = single_log_line(Some(model)),
        custom_llm_provider = single_log_line(custom_llm_provider),
        "OCR batch cost: no pricing entry; billing only the families the deployment prices"
    );
}

pub(super) fn warn_batch_missing_page_rate(
    model: &str,
    custom_llm_provider: Option<&str>,
    pages_processed: i64,
) {
    tracing::warn!(
        model = single_log_line(Some(model)),
        custom_llm_provider = single_log_line(custom_llm_provider),
        pages_processed = pages_processed,
        "OCR batch cost: no ocr_cost_per_page is configured; returning 0.0 cost for those pages"
    );
}
