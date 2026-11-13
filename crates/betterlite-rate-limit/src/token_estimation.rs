pub mod audio;
pub mod call_family;
pub mod estimate;
pub mod google;
pub mod input_text;
pub mod output_cap;
pub mod services;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// the request dictionary every pre-call hook receives, keyed the way LiteLLM's `data` is
pub type RequestData = IndexMap<String, Json>;

#[path = "token_estimation/tests/apply_implicit_cap.rs"]
mod apply_implicit_cap;
#[path = "token_estimation/tests/apply_implicit_cap_skip.rs"]
mod apply_implicit_cap_skip;
#[path = "token_estimation/tests/audio_block.rs"]
mod audio_block;
#[path = "token_estimation/tests/audio_content.rs"]
mod audio_content;
#[path = "token_estimation/tests/candidate_count.rs"]
mod candidate_count;
#[path = "token_estimation/tests/candidate_multiply.rs"]
mod candidate_multiply;
#[path = "token_estimation/tests/combined_tokens.rs"]
mod combined_tokens;
#[path = "token_estimation/tests/embedding_detect.rs"]
mod embedding_detect;
#[path = "token_estimation/tests/explicit_output_cap.rs"]
mod explicit_output_cap;
#[path = "token_estimation/tests/google_native.rs"]
mod google_native;
#[path = "token_estimation/tests/has_explicit_cap.rs"]
mod has_explicit_cap;
#[path = "token_estimation/tests/input_output.rs"]
mod input_output;
#[path = "token_estimation/tests/no_max_tokens_floor.rs"]
mod no_max_tokens_floor;
#[path = "token_estimation/tests/parse_output_cap.rs"]
mod parse_output_cap;
#[path = "token_estimation/tests/precise_input.rs"]
mod precise_input;
#[path = "token_estimation/tests/precise_input_counter.rs"]
mod precise_input_counter;
#[path = "token_estimation/tests/pretokenized.rs"]
mod pretokenized;
#[path = "token_estimation/tests/rerank_text.rs"]
mod rerank_text;
#[path = "token_estimation/tests/responses_to_chat.rs"]
mod responses_to_chat;
#[path = "token_estimation/tests/strip_audio.rs"]
mod strip_audio;
#[path = "token_estimation/tests/support.rs"]
mod support;
