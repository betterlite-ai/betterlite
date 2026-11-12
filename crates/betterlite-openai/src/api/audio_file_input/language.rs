const BARE_ISO_639_1_TO_BCP47: &[(&str, &str)] = &[
    ("en", "en-US"),
    ("es", "es-ES"),
    ("de", "de-DE"),
    ("fr", "fr-FR"),
    ("it", "it-IT"),
    ("pt", "pt-BR"),
    ("ja", "ja-JP"),
    ("ko", "ko-KR"),
    ("zh", "zh-CN"),
    ("ru", "ru-RU"),
    ("hi", "hi-IN"),
    ("ar", "ar-SA"),
];

#[must_use]
pub fn normalize_language_to_bcp47(language: &str) -> String {
    if language.contains('-') {
        return language.to_owned();
    }
    let bare = language.to_lowercase();
    BARE_ISO_639_1_TO_BCP47
        .iter()
        .find_map(|(code, tag)| (*code == bare.as_str()).then_some(*tag))
        .map_or_else(|| language.to_owned(), str::to_owned)
}
