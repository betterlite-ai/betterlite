use betterlite_openai::search::records::SearchResponse;

#[must_use]
pub fn create_empty_search_result() -> (String, Option<SearchResponse>) {
    const NO_SEARCH_QUERY: &str = "No search query provided";
    (NO_SEARCH_QUERY.to_owned(), None)
}
