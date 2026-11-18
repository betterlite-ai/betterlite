use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;

use crate::enterprise_ml_dispatch::sagemaker::Tokenizer;

#[test]
fn tokenizer_counts_tokens() {
    let tokenizer = Tokenizer::new().expect("the bundled vocabulary loads");
    assert_eq!(tokenizer.count_response_tokens("hello world"), 2);
    assert_eq!(
        tokenizer
            .count_tokens("hello world")
            .expect("the engine encodes"),
        2
    );
}
