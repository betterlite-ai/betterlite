use betterlite_openapi::document_builder::RouteDoc;

mod tests;

// LiteLLM marks every route in this router include_in_schema=False, so it documents none of them
pub const DOCUMENTED_ROUTES: &[RouteDoc] = &[];
