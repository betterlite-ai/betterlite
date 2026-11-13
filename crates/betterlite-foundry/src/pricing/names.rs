const FEE_LOOKUP_FALLBACK: &str = "model_router";
const PROVIDER_PREFIX: &str = "azure_ai/";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterName {
    Plain,
    RouterShaped,
    HyphenatedEntry,
    UnderscoredEntry,
}

impl RouterName {
    #[must_use]
    pub const fn is_router_shaped(self) -> bool {
        match self {
            Self::Plain => false,
            Self::RouterShaped | Self::HyphenatedEntry | Self::UnderscoredEntry => true,
        }
    }

    #[must_use]
    pub const fn is_fee_entry(self) -> bool {
        match self {
            Self::HyphenatedEntry | Self::UnderscoredEntry => true,
            Self::Plain | Self::RouterShaped => false,
        }
    }

    #[must_use]
    pub const fn fee_lookup_name(self) -> &'static str {
        match self {
            Self::HyphenatedEntry => "model-router",
            Self::Plain | Self::RouterShaped | Self::UnderscoredEntry => FEE_LOOKUP_FALLBACK,
        }
    }
}

// Any position counts as router-shaped, but only the exact name after one azure_ai/ strip is a fee entry.
#[must_use]
pub fn classify_router_name(model: &str) -> RouterName {
    let lowered = model.to_lowercase();
    let entry = lowered.strip_prefix(PROVIDER_PREFIX).unwrap_or(&lowered);
    if entry == "model-router" {
        return RouterName::HyphenatedEntry;
    }
    if entry == "model_router" {
        return RouterName::UnderscoredEntry;
    }
    if lowered.contains("model-router") || lowered.contains("model_router") {
        return RouterName::RouterShaped;
    }
    RouterName::Plain
}

// The response name is the model whose tokens were priced, so it outranks the requested router.
#[must_use]
pub fn router_fee_name<'request>(
    model: &'request str,
    request_model: Option<&'request str>,
) -> Option<&'request str> {
    let name = classify_router_name(model);
    if name.is_fee_entry() {
        return None;
    }
    if name.is_router_shaped() {
        return Some(model);
    }
    request_model.filter(|candidate| classify_router_name(candidate).is_router_shaped())
}
