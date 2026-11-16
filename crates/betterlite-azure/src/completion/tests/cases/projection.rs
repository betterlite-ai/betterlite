use betterlite_models::decisions::{Extra, Json};

use crate::completion::config::AzureOpenAITextConfig;
use crate::completion::error::STREAM_ANSWER;
use crate::completion::tests::fixture::{Answer, CLOCK};
use crate::endpoints::projection::ClientKind;

use super::support::{MODEL, Scenario, at, count, float, number, stream_items, text};

const TWO_CHOICES: &str = r#"{"id":"cmpl-9","object":"text_completion","created":1234,"model":"gpt-35-turbo-instruct","system_fingerprint":"fp_1","choices":[{"text":"A","index":5,"logprobs":null,"finish_reason":"length"},{"text":"B","index":9,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;
const NO_USAGE: &str = r#"{"id":"cmpl-2","object":"text_completion","created":777,"model":"gpt-35-turbo-instruct","choices":[{"text":"x","index":0,"finish_reason":"content_filter"}],"usage":null}"#;
const EXTRA_USAGE: &str = r#"{"id":"cmpl-3","object":"text_completion","created":888,"model":"gpt-35-turbo-instruct","choices":[{"text":"y","index":0,"finish_reason":"stop","logprobs":null}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null,"reasoning_tokens":2,"queue_time":0.25}}"#;

fn projected(kind: ClientKind, body: &str) -> Json {
    let answer = Answer::Buffered(body.to_owned());
    Scenario::new(kind, answer, "{}").execute().reply_json()
}

fn settings_json(config: &AzureOpenAITextConfig) -> Json {
    let value = serde_json::to_value(config.settings().unwrap()).unwrap();
    serde_json::from_value(value).unwrap()
}

fn config_forwards_its_eight_parameters() {
    let none = settings_json(&AzureOpenAITextConfig::default());
    assert_eq!(none, Json::Object(Extra::new()));
    let config = AzureOpenAITextConfig {
        frequency_penalty: Some(count(1)),
        logit_bias: Some(count(2)),
        max_tokens: Some(count(3)),
        number: Some(count(4)),
        presence_penalty: Some(count(5)),
        stop: Some(Json::Text("END".to_owned())),
        temperature: Some(count(6)),
        top_p: Some(count(7)),
    };
    let expected = Extra::from_iter([
        ("frequency_penalty".to_owned(), count(1)),
        ("logit_bias".to_owned(), count(2)),
        ("max_tokens".to_owned(), count(3)),
        ("n".to_owned(), count(4)),
        ("presence_penalty".to_owned(), count(5)),
        ("stop".to_owned(), Json::Text("END".to_owned())),
        ("temperature".to_owned(), count(6)),
        ("top_p".to_owned(), count(7)),
    ]);
    assert_eq!(settings_json(&config), Json::Object(expected));
}

fn sync_choices() {
    let reply = projected(ClientKind::AzureOpenAi, TWO_CHOICES);
    assert_eq!(text(&at(&reply, "id")), "cmpl-9");
    assert_eq!(text(&at(&reply, "object")), "chat.completion");
    assert_eq!(number(&at(&reply, "created")), CLOCK);
    assert_eq!(text(&at(&reply, "model")), MODEL);
    assert_eq!(at(&reply, "system_fingerprint"), Json::Null);
    let Json::List(choices) = at(&reply, "choices") else {
        panic!("a list of choices");
    };
    assert_eq!(choices.len(), 2);
    sync_choice_entries(&reply);
}

fn sync_choice_entries(reply: &Json) {
    assert_eq!(text(&at(reply, "choices.0.message.content")), "A");
    assert_eq!(text(&at(reply, "choices.0.message.role")), "assistant");
    assert_eq!(at(reply, "choices.0.message.tool_calls"), Json::Null);
    assert_eq!(at(reply, "choices.0.message.function_call"), Json::Null);
    assert_eq!(text(&at(reply, "choices.0.finish_reason")), "length");
    assert_eq!(number(&at(reply, "choices.0.index")), 0);
    let Json::Object(first) = at(reply, "choices.0") else {
        panic!("a choice object");
    };
    assert!(!first.contains_key("logprobs"));
    assert_eq!(text(&at(reply, "choices.1.message.content")), "B");
    assert_eq!(text(&at(reply, "choices.1.finish_reason")), "stop");
    assert_eq!(number(&at(reply, "choices.1.index")), 1);
}

fn sync_usage_and_original() {
    let reply = projected(ClientKind::AzureOpenAi, TWO_CHOICES);
    assert_eq!(number(&at(&reply, "usage.prompt_tokens")), 3);
    assert_eq!(number(&at(&reply, "usage.total_tokens")), 5);
    assert_eq!(at(&reply, "usage.completion_tokens_details"), Json::Null);
    assert_eq!(at(&reply, "usage.prompt_tokens_details"), Json::Null);
    let original = "_hidden_params.original_response";
    assert_eq!(
        text(&at(&reply, &format!("{original}.object"))),
        "text_completion"
    );
    assert_eq!(number(&at(&reply, &format!("{original}.created"))), 1234);
    let finish = format!("{original}.choices.0.finish_reason");
    assert_eq!(text(&at(&reply, &finish)), "length");
}

fn async_raw_usage() {
    let reply = projected(ClientKind::AsyncAzureOpenAi, TWO_CHOICES);
    assert_eq!(at(&reply, "usage.total_tokens"), count(5));
    let Json::Object(usage) = at(&reply, "usage") else {
        panic!("the raw usage object");
    };
    assert_eq!(usage.get("completion_tokens_details"), Some(&Json::Null));
    let original = "_hidden_params.original_response";
    assert_eq!(
        text(&at(&reply, &format!("{original}.system_fingerprint"))),
        "fp_1"
    );
    let finish = format!("{original}.choices.0.finish_reason");
    assert_eq!(text(&at(&reply, &finish)), "length");
}

fn usage_variants() {
    let async_reply = projected(ClientKind::AsyncAzureOpenAi, EXTRA_USAGE);
    assert_eq!(number(&at(&async_reply, "usage.reasoning_tokens")), 2);
    assert_eq!(
        at(&async_reply, "usage.completion_tokens_details"),
        Json::Null
    );
    let sync_reply = projected(ClientKind::AzureOpenAi, EXTRA_USAGE);
    let Json::Object(usage) = at(&sync_reply, "usage") else {
        panic!("the sync usage object");
    };
    assert!(!usage.contains_key("reasoning_tokens"));
    let reasoning = at(
        &sync_reply,
        "usage.completion_tokens_details.reasoning_tokens",
    );
    assert_eq!(number(&reasoning), 2);
    let text_tokens = at(&sync_reply, "usage.completion_tokens_details.text_tokens");
    assert_eq!(number(&text_tokens), 0);
    assert_eq!(at(&sync_reply, "usage.queue_time"), float(0.25));
}

fn missing_usage() {
    let sync_reply = projected(ClientKind::AzureOpenAi, NO_USAGE);
    assert_eq!(number(&at(&sync_reply, "usage.total_tokens")), 0);
    assert_eq!(
        text(&at(&sync_reply, "choices.0.finish_reason")),
        "content_filter"
    );
    let async_reply = projected(ClientKind::AsyncAzureOpenAi, NO_USAGE);
    let Json::Object(fields) = &async_reply else {
        panic!("a projected object");
    };
    assert_eq!(fields.get("usage"), Some(&Json::Null));
    let Json::Object(original) = at(&async_reply, "_hidden_params.original_response") else {
        panic!("the raw original response");
    };
    assert_eq!(original.get("usage"), Some(&Json::Null));
}

fn stream_answer_to_a_buffered_request() {
    for kind in [ClientKind::AzureOpenAi, ClientKind::AsyncAzureOpenAi] {
        let answer = Answer::Stream(stream_items());
        let run = Scenario::new(kind, answer, "{}").execute();
        assert_eq!(run.failure().status, Some(500));
        assert_eq!(run.failure().message, STREAM_ANSWER);
        assert_eq!(run.hooks.pre.len(), 1);
    }
}

pub(super) fn azure_text_response_projection() {
    config_forwards_its_eight_parameters();
    sync_choices();
    sync_usage_and_original();
    async_raw_usage();
    usage_variants();
    missing_usage();
    stream_answer_to_a_buffered_request();
}
