use betterlite_openapi::document_builder::RouteDoc;

mod tests;

// LiteLLM marks every route in this router include_in_schema=False, so it contributes no paths
#[must_use]
pub const fn documented_routes() -> Vec<RouteDoc> {
    Vec::new()
}
