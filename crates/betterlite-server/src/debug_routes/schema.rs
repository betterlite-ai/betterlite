use betterlite_openapi::document_builder::RouteDoc;

mod tests;

// every route is include_in_schema=False, so the empty list and its mutant replacement are equal
#[cfg_attr(test, mutants::skip)]
#[must_use]
pub const fn documented_routes() -> Vec<RouteDoc> {
    Vec::new()
}
