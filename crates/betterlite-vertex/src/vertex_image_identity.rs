pub mod error;
pub mod settings;
pub mod target;

mod tests;

// The five OpenAI sizes the Vertex image adapters understand, in LiteLLM's map order
pub const SIZE_ASPECT_RATIOS: [(&str, &str); 5] = [
    ("1024x1024", "1:1"),
    ("1792x1024", "16:9"),
    ("1024x1792", "9:16"),
    ("1280x896", "4:3"),
    ("896x1280", "3:4"),
];

// Every other size, absent or unknown, takes 1:1 rather than a nearest-aspect guess
#[must_use]
pub fn size_aspect_ratio(size: &str) -> &'static str {
    SIZE_ASPECT_RATIOS
        .iter()
        .find_map(|(openai, ratio)| (*openai == size).then_some(*ratio))
        .unwrap_or("1:1")
}
