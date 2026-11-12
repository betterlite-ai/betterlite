#[path = "keywords/tests.rs"]
mod tests;

const VERBS: [&str; 11] = [
    "Ignore",
    "Disregard",
    "Skip",
    "Forget",
    "Neglect",
    "Overlook",
    "Omit",
    "Bypass",
    "Pay no attention to",
    "Do not follow",
    "Do not obey",
];

const ADJECTIVES: [&str; 8] = [
    "",
    "prior",
    "previous",
    "preceding",
    "above",
    "foregoing",
    "earlier",
    "initial",
];

const PREPOSITIONS: [&str; 5] = [
    "",
    "and start over",
    "and start anew",
    "and begin afresh",
    "and start from scratch",
];

#[must_use]
pub fn generate_injection_keywords() -> Vec<String> {
    VERBS
        .iter()
        .flat_map(|verb| {
            ADJECTIVES.iter().flat_map(move |adjective| {
                PREPOSITIONS
                    .iter()
                    .map(move |preposition| [*verb, *adjective, *preposition])
            })
        })
        .map(|parts| {
            parts
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<&str>>()
                .join(" ")
        })
        .map(|phrase| phrase.trim().to_lowercase())
        .filter(|phrase| phrase.split_whitespace().count() > 2)
        .collect()
}
