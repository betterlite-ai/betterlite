mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocsEnv {
    pub docs_url: Option<String>,
    pub redoc_url: Option<String>,
    pub openapi_url: Option<String>,
    pub no_docs: Option<String>,
    pub no_redoc: Option<String>,
    pub no_openapi: Option<String>,
    pub root_redirect_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocsUrls {
    pub docs_url: Option<String>,
    pub redoc_url: Option<String>,
    pub openapi_url: Option<String>,
    pub root_redirect_url: Option<String>,
}

impl DocsEnv {
    #[must_use]
    pub fn resolve(&self) -> DocsUrls {
        DocsUrls {
            docs_url: resolved_url(self.docs_url.as_deref(), self.no_docs.as_deref(), "/"),
            redoc_url: resolved_url(
                self.redoc_url.as_deref(),
                self.no_redoc.as_deref(),
                "/redoc",
            ),
            openapi_url: resolved_url(
                self.openapi_url.as_deref(),
                self.no_openapi.as_deref(),
                "/openapi.json",
            ),
            // LiteLLM reads ROOT_REDIRECT_URL with getenv alone, so an empty value still registers the route
            root_redirect_url: self.root_redirect_url.clone(),
        }
    }
}

fn resolved_url(explicit: Option<&str>, disabled: Option<&str>, default: &str) -> Option<String> {
    if let Some(url) = explicit.filter(|url| !url.is_empty()) {
        return Some(url.to_owned());
    }
    if disabled.is_some_and(|text| text.trim().to_lowercase() == "true") {
        return None;
    }
    Some(default.to_owned())
}
