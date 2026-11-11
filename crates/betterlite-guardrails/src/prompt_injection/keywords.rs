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
    let mut keywords = Vec::new();
    for verb in VERBS {
        for adjective in ADJECTIVES {
            for preposition in PREPOSITIONS {
                let phrase = [verb, adjective, preposition]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<&str>>()
                    .join(" ");
                let phrase = phrase.trim();
                if phrase.split_whitespace().count() > 2 {
                    keywords.push(phrase.to_lowercase());
                }
            }
        }
    }
    keywords
}
