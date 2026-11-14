use super::*;

#[tokio::test]
async fn maps_extracted_paths_to_the_full_content() {
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "# main.py\nprint('hi')"}),
        json!({"role": "user", "content": "last question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(
        result.cache.get("main.py").map(String::as_str),
        Some("# main.py\nprint('hi')")
    );
}

#[tokio::test]
async fn suffixes_duplicate_keys() {
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "# main.py\nfirst"}),
        json!({"role": "user", "content": "# main.py\nsecond"}),
        json!({"role": "user", "content": "last question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    let keys: Vec<&String> = result.cache.keys().collect();
    assert_eq!(keys, vec!["main.py", "main.py_2"]);
}

#[tokio::test]
async fn falls_back_to_the_message_index() {
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "no path here"}),
        json!({"role": "user", "content": "last question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(
        result.cache.get("message_1").map(String::as_str),
        Some("no path here")
    );
}

#[tokio::test]
async fn uses_the_text_of_block_content() {
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": [
            {"type": "text", "text": "# app.ts"},
            {"type": "image", "source": {"type": "base64", "data": "AAAA"}}
        ]}),
        json!({"role": "user", "content": "last question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(
        result.cache.get("app.ts").map(String::as_str),
        Some("# app.ts")
    );
}
