use crate::engine::tiktoken::Encoding;
// tiktoken 0.14.0 model.py MODEL_TO_ENCODING, in its own order
const MODEL_ENCODINGS: [(&str, Encoding); 45] = [
    ("o1", Encoding::O200kBase),
    ("o3", Encoding::O200kBase),
    ("o4-mini", Encoding::O200kBase),
    ("gpt-5", Encoding::O200kBase),
    ("gpt-4.1", Encoding::O200kBase),
    ("gpt-4o", Encoding::O200kBase),
    ("gpt-4", Encoding::Cl100kBase),
    ("gpt-3.5-turbo", Encoding::Cl100kBase),
    ("gpt-3.5", Encoding::Cl100kBase),
    ("gpt-35-turbo", Encoding::Cl100kBase),
    ("davinci-002", Encoding::Cl100kBase),
    ("babbage-002", Encoding::Cl100kBase),
    ("text-embedding-ada-002", Encoding::Cl100kBase),
    ("text-embedding-3-small", Encoding::Cl100kBase),
    ("text-embedding-3-large", Encoding::Cl100kBase),
    ("text-davinci-003", Encoding::P50kBase),
    ("text-davinci-002", Encoding::P50kBase),
    ("text-davinci-001", Encoding::R50kBase),
    ("text-curie-001", Encoding::R50kBase),
    ("text-babbage-001", Encoding::R50kBase),
    ("text-ada-001", Encoding::R50kBase),
    ("davinci", Encoding::R50kBase),
    ("curie", Encoding::R50kBase),
    ("babbage", Encoding::R50kBase),
    ("ada", Encoding::R50kBase),
    ("code-davinci-002", Encoding::P50kBase),
    ("code-davinci-001", Encoding::P50kBase),
    ("code-cushman-002", Encoding::P50kBase),
    ("code-cushman-001", Encoding::P50kBase),
    ("davinci-codex", Encoding::P50kBase),
    ("cushman-codex", Encoding::P50kBase),
    ("text-davinci-edit-001", Encoding::P50kEdit),
    ("code-davinci-edit-001", Encoding::P50kEdit),
    ("text-similarity-davinci-001", Encoding::R50kBase),
    ("text-similarity-curie-001", Encoding::R50kBase),
    ("text-similarity-babbage-001", Encoding::R50kBase),
    ("text-similarity-ada-001", Encoding::R50kBase),
    ("text-search-davinci-doc-001", Encoding::R50kBase),
    ("text-search-curie-doc-001", Encoding::R50kBase),
    ("text-search-babbage-doc-001", Encoding::R50kBase),
    ("text-search-ada-doc-001", Encoding::R50kBase),
    ("code-search-babbage-code-001", Encoding::R50kBase),
    ("code-search-ada-code-001", Encoding::R50kBase),
    ("gpt2", Encoding::Gpt2),
    ("gpt-2", Encoding::Gpt2),
];
// tiktoken 0.14.0 model.py MODEL_PREFIX_TO_ENCODING, first match wins
const MODEL_PREFIX_ENCODINGS: [(&str, Encoding); 17] = [
    ("o1-", Encoding::O200kBase),
    ("o3-", Encoding::O200kBase),
    ("o4-mini-", Encoding::O200kBase),
    ("gpt-5", Encoding::O200kBase),
    ("gpt-4.5-", Encoding::O200kBase),
    ("gpt-4.1-", Encoding::O200kBase),
    ("chatgpt-4o-", Encoding::O200kBase),
    ("gpt-4o-", Encoding::O200kBase),
    ("gpt-4-", Encoding::Cl100kBase),
    ("gpt-3.5-turbo-", Encoding::Cl100kBase),
    ("gpt-35-turbo-", Encoding::Cl100kBase),
    ("gpt-oss-", Encoding::O200kHarmony),
    ("ft:gpt-4o", Encoding::O200kBase),
    ("ft:gpt-4", Encoding::Cl100kBase),
    ("ft:gpt-3.5-turbo", Encoding::Cl100kBase),
    ("ft:davinci-002", Encoding::Cl100kBase),
    ("ft:babbage-002", Encoding::Cl100kBase),
];

pub(super) fn tiktoken_encoding_name(model: &str) -> Option<Encoding> {
    MODEL_ENCODINGS
        .iter()
        .find(|(name, _)| *name == model)
        .or_else(|| {
            MODEL_PREFIX_ENCODINGS
                .iter()
                .find(|(prefix, _)| model.starts_with(prefix))
        })
        .map(|(_, encoding)| *encoding)
}
