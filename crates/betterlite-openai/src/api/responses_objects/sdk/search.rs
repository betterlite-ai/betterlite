use std::collections::BTreeMap;

record! { FileSearch: FileSearchTag = "file_search_call" {
    id: String,
    queries: Vec<String>,
    status: super::literals::FileSearchStatusChoice,
    results: Option<Vec<FileSearchResult>>,
} }
record! { FileSearchResult {
    attributes: Option<BTreeMap<String, super::unions::FileSearchResultAttributeValue>>,
    file_id: Option<String>,
    filename: Option<String>,
    score: Option<super::Number>,
    text: Option<String>,
} }
record! { WebSearch: WebSearchTag = "web_search_call" {
    id: String,
    action: super::unions::WebSearchActionValue,
    status: super::literals::WebSearchStatusChoice,
} }
record! { ActionSearch: ActionSearchTag = "search" {
    queries: Option<Vec<String>>,
    query: Option<String>,
    sources: Option<Vec<ActionSearchSource>>,
} }
record! { ActionSearchSource: ActionSearchSourceTag = "url" {
    url: String,
} }
record! { ActionOpenPage: ActionOpenPageTag = "open_page" {
    url: Option<String>,
} }
record! { ActionFind: ActionFindTag = "find_in_page" {
    pattern: String,
    url: String,
} }
record! { OutputFileSearch: OutputFileSearchTag = "file_search" {
    vector_store_ids: Vec<String>,
    filters: Option<super::unions::OutputFileSearchFiltersValue>,
    max_num_results: Option<i64>,
    ranking_options: Option<RankingOptions>,
} }
record! { RankingOptions {
    hybrid_search: Option<RankingOptionsHybridSearch>,
    ranker: Option<super::literals::RankingOptionsRankerChoice>,
    score_threshold: Option<super::Number>,
} }
record! { RankingOptionsHybridSearch {
    embedding_weight: super::Number,
    text_weight: super::Number,
} }
record! { OutputWebSearch {
    r#type: super::literals::OutputWebSearchTypeChoice,
    filters: Option<Filters>,
    search_context_size: Option<super::literals::OutputWebSearchSearchConChoice>,
    user_location: Option<UserLocation>,
} }
record! { Filters {
    allowed_domains: Option<Vec<String>>,
} }
record! { UserLocation {
    city: Option<String>,
    country: Option<String>,
    region: Option<String>,
    timezone: Option<String>,
    r#type: Option<super::literals::ApproximateKind>,
} }
record! { WebSearchPreview {
    r#type: super::literals::WebSearchPreviewTypeChoice,
    search_content_types: Option<Vec<super::literals::WebSearchPreviewSearchCoChoice>>,
    search_context_size: Option<super::literals::OutputWebSearchSearchConChoice>,
    user_location: Option<OutputUserLocation>,
} }
record! { OutputUserLocation: OutputUserLocationTag = "approximate" {
    city: Option<String>,
    country: Option<String>,
    region: Option<String>,
    timezone: Option<String>,
} }
