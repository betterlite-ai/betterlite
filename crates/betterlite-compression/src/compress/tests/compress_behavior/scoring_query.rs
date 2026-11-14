use super::super::*;

#[tokio::test]
async fn scores_candidates_with_the_last_user_message() {
    let messages = vec![
        json!({"role": "system", "content": "rules"}),
        json!({"role": "user", "content": "beta beta beta"}),
        json!({"role": "user", "content": "alpha alpha alpha"}),
        json!({"role": "user", "content": "alpha question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(6)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages[2], messages[2]);
    assert_eq!(
        result.cache,
        IndexMap::from([("message_1".to_owned(), "beta beta beta".to_owned())])
    );
}
