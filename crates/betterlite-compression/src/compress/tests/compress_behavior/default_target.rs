use super::super::*;
use rstest::rstest;

#[rstest]
#[case::ten_token_trigger_fits(10_usize, 2_usize, true)]
#[case::ten_token_trigger_drops(10, 3, false)]
#[case::hundred_token_trigger_fits(100, 65, true)]
#[case::hundred_token_trigger_drops(100, 66, false)]
#[case::fifteen_token_trigger_fits(15, 5, true)]
#[case::fifteen_token_trigger_drops(15, 6, false)]
#[tokio::test]
async fn is_seventy_percent_of_the_trigger(
    #[case] trigger: usize,
    #[case] candidate_words: usize,
    #[case] kept: bool,
) {
    let messages = vec![
        json!({"role": "system", "content": "s1 s2 s3"}),
        json!({"role": "user", "content": words(candidate_words)}),
        json!({"role": "assistant", "content": "a1"}),
        json!({"role": "user", "content": "u1"}),
    ];
    let counter = fixture_counter(vec![trigger + 1]);
    let result = compress(&request(&messages, trigger, None), &counter)
        .await
        .expect("compressed");
    if kept {
        assert_eq!(result.messages[1], messages[1]);
        assert!(result.cache.is_empty());
    } else {
        assert!(
            result.messages[1]["content"]
                .as_str()
                .is_some_and(|text| text.starts_with("[Compressed:"))
        );
        assert_eq!(result.cache.len(), 1);
    }
}
