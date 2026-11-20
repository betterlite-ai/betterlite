#![cfg_attr(
    test,
    expect(
        clippy::cognitive_complexity,
        clippy::too_many_lines,
        reason = "one scenario function per oracle case reads top to bottom as its fixture steps"
    )
)]

use betterlite_openai::openai_base_config::Headers;
use tracing::Level;

use crate::mantle_settings::chat::{MantleChatConfig, supports_reasoning};

use super::support::{self, Answer, Policy};

pub(super) fn chat_provider_prefix() {
    let config = MantleChatConfig::new(None);
    // Inherited GPT validation sets Content-Type, then a supplied key, and leaves caller headers alone.
    let empty = config.validate_environment(None, &Headers::new(), None);
    assert_eq!(
        support::header(&empty, "Content-Type").as_deref(),
        Some("application/json")
    );
    assert_eq!(support::header(&empty, "Authorization"), None);
    let key = support::secret("explicit-fixture-key");
    let keyed = config.validate_environment(Some(&key), &Headers::new(), None);
    assert_eq!(
        support::header(&keyed, "Authorization").as_deref(),
        Some("Bearer explicit-fixture-key")
    );
    let caller = support::headers(&[
        ("content-type", "application/vnd.fixture+json"),
        ("Authorization", "Bearer caller-fixture-key"),
    ]);
    let untouched = config.validate_environment(None, &caller, None);
    assert_eq!(
        support::header(&untouched, "content-type").as_deref(),
        Some("application/vnd.fixture+json")
    );
    assert_eq!(support::header(&untouched, "Content-Type"), None);
    assert_eq!(
        support::header(&untouched, "Authorization").as_deref(),
        Some("Bearer caller-fixture-key")
    );
    // A truthy aws_bedrock_project_id overwrites the exact OpenAI-Project header; an empty one leaves the headers unchanged.
    let existing = support::headers(&[("OpenAI-Project", "guessed-project")]);
    let projected = config.validate_environment(None, &existing, Some("fixture-project"));
    assert_eq!(
        support::header(&projected, "OpenAI-Project").as_deref(),
        Some("fixture-project")
    );
    let blank = config.validate_environment(None, &existing, Some(""));
    assert_eq!(
        support::header(&blank, "OpenAI-Project").as_deref(),
        Some("guessed-project")
    );
    // Supported params: the inherited GPT list, then verbosity for a gpt reasoning series name, then reasoning_effort when the policy says so.
    let catalog = support::catalog(&["gpt-5", "gpt-4"]);
    let mut unsupported = Policy::new(Answer::Unsupported);
    let gpt5 = support::block_on(config.supported_params("gpt-5", &catalog, &mut unsupported));
    assert!(gpt5.contains(&String::from("verbosity")));
    assert!(!gpt5.contains(&String::from("reasoning_effort")));
    assert!(gpt5.contains(&String::from("user")));
    assert!(gpt5.contains(&String::from("response_format")));
    assert_eq!(
        unsupported.requests.seen(),
        vec![(
            String::from("gpt-5"),
            Some(String::from("bedrock_mantle")),
            String::from("supports_reasoning")
        )]
    );
    let mut supported = Policy::new(Answer::Supported);
    let oss =
        support::block_on(config.supported_params("openai.gpt-oss-120b", &catalog, &mut supported));
    assert_eq!(oss.last().map(String::as_str), Some("reasoning_effort"));
    assert!(!oss.contains(&String::from("verbosity")));
    assert!(!oss.contains(&String::from("user")));
    let gpt4 = support::block_on(config.supported_params("gpt-4", &catalog, &mut supported));
    assert!(gpt4.contains(&String::from("user")));
    assert!(!gpt4.contains(&String::from("response_format")));
    assert!(!gpt4.contains(&String::from("verbosity")));
    assert_eq!(gpt4.last().map(String::as_str), Some("reasoning_effort"));
    let chat = support::block_on(config.supported_params("gpt-5-chat", &catalog, &mut supported));
    assert!(!chat.contains(&String::from("verbosity")));
    // A failing policy is logged at debug and reads as unsupported.
    let (messages, _guard) = support::capture();
    let mut panicking = Policy::new(Answer::Panic);
    assert!(!support::block_on(supports_reasoning(
        "gpt-5",
        &mut panicking
    )));
    assert_eq!(
        messages.at(Level::DEBUG),
        vec![String::from(
            "BedrockMantleChatConfig: error checking reasoning support: fixture reasoning probe"
        )]
    );
    // get_error_class keeps the supplied fields.
    let error = config
        .get_error_class(
            "quota exhausted",
            429,
            &support::headers(&[("retry-after", "7")]),
        )
        .expect("the error builds");
    assert_eq!(error.error.status_code, 429);
    assert_eq!(error.error.message, "quota exhausted");
    assert_eq!(
        error.error.headers,
        Some(vec![(String::from("retry-after"), String::from("7"))])
    );
    // The stream iterator keeps json_mode only; sync_stream has no effect.
    assert!(!MantleChatConfig::response_iterator(true, None).json_mode);
    assert!(MantleChatConfig::response_iterator(false, Some(true)).json_mode);
    assert!(!MantleChatConfig::response_iterator(false, Some(false)).json_mode);
}
