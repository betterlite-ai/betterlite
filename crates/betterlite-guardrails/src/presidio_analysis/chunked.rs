use futures_util::future::join_all;

use super::error::PresidioAnalysisError;
use super::filter::filter_by_score;
use super::merge::merge_chunked_analyze_results;
use super::split::split_text_for_analysis;
use super::wire::AnalyzeResponseItem;
use super::{CHUNK_OVERLAP_CHARS, PresidioAnalyzer, RequestOverrides};

pub(super) async fn analyze_text_chunked(
    analyzer: &PresidioAnalyzer,
    text: &str,
    request: &RequestOverrides<'_>,
) -> Result<Vec<AnalyzeResponseItem>, PresidioAnalysisError> {
    let settings = &analyzer.settings;
    let text_chunks = split_text_for_analysis(text, settings.chunk_size_bytes, CHUNK_OVERLAP_CHARS);
    tracing::debug!(
        chunk_size_bytes = settings.chunk_size_bytes,
        chunks = text_chunks.len(),
        "Presidio analyze: text exceeds the chunk size, analyzing in overlapping chunks"
    );
    let semaphore = analyzer.chunk_semaphore()?;
    let gathered = join_all(text_chunks.iter().map(|chunk| async {
        let _permit = semaphore
            .acquire()
            .await
            .map_err(|source| PresidioAnalysisError::SemaphoreClosed { source })?;
        analyzer.analyze_single(&chunk.text, request).await
    }))
    .await;
    let mut chunk_results = Vec::with_capacity(gathered.len());
    for result in gathered {
        // Filtering before the merge keeps a below-threshold detection from winning an overlap against one that is kept.
        chunk_results.push(filter_by_score(settings, result?));
    }
    Ok(merge_chunked_analyze_results(&text_chunks, chunk_results))
}
