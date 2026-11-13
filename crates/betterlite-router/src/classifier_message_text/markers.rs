#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkerPair<'marker> {
    pub open: &'marker str,
    pub close: &'marker str,
}

pub const DEFAULT_REMINDER_MARKERS: &[MarkerPair<'static>] = &[MarkerPair {
    open: "<system-reminder>",
    close: "</system-reminder>",
}];

pub const CODEX_REMINDER_MARKERS: &[MarkerPair<'static>] = &[
    MarkerPair {
        open: "<system-reminder>",
        close: "</system-reminder>",
    },
    MarkerPair {
        open: "<environment_context>",
        close: "</environment_context>",
    },
    MarkerPair {
        open: "<recommended_plugins>",
        close: "</recommended_plugins>",
    },
    MarkerPair {
        open: "<user_instructions>",
        close: "</user_instructions>",
    },
    MarkerPair {
        open: "<environments_instructions>",
        close: "</environments_instructions>",
    },
    MarkerPair {
        open: "# agents.md instructions for ",
        close: "</instructions>",
    },
];

// LiteLLM's client prefix carries a case-insensitive ASCII regex, so upper case counts too
#[must_use]
pub fn is_codex_user_agent(user_agent: &str) -> bool {
    let mut characters = user_agent.chars();
    let prefix: String = characters.by_ref().take(5).collect();
    prefix.eq_ignore_ascii_case("codex") && matches!(characters.next(), Some('-' | '_' | ' ' | '/'))
}

#[must_use]
pub fn reminder_markers_for_request<'request>(
    configured: Option<&'request [MarkerPair<'request>]>,
    codex_user_agent: bool,
) -> &'request [MarkerPair<'request>] {
    match configured {
        Some(markers) => markers,
        None if codex_user_agent => CODEX_REMINDER_MARKERS,
        None => DEFAULT_REMINDER_MARKERS,
    }
}
