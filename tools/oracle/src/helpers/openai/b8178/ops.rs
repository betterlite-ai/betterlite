use betterlite_openai::gigachat_embedding::{
    EmbeddingRequestInput, embedding_request, transform_response,
};
use betterlite_openai::gigachat_passthrough::{
    GigachatPassthroughConfig, GigachatPassthroughLogged, validate_environment,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest, RelayedResponse,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::input::{
    EmbeddingReplyStep, EmbeddingRequestStep, PassthroughCollectedStep, PassthroughHeadersStep,
    PassthroughKeyStep, PassthroughLoggingStep, PassthroughStreamingStep, PassthroughUrlStep, Step,
};
use super::support::{
    NoopLogging, TokenBearer, chat_value, collected_value, embedding_value, error_status,
};

pub(crate) fn execute(step: &Step, runtime: &Runtime) -> Value {
    match step {
        Step::EmbeddingRequest(value) => embedding_request_value(runtime, value),
        Step::EmbeddingReply(value) => embedding_reply_value(value),
        Step::PassthroughUrl(value) => passthrough_url_value(value),
        Step::PassthroughHeaders(value) => passthrough_headers_value(runtime, value),
        Step::PassthroughKey(value) => passthrough_key_value(value),
        Step::PassthroughStreaming(value) => passthrough_streaming_value(value),
        Step::PassthroughLogging(value) => passthrough_logging_value(value),
        Step::PassthroughCollected(value) => passthrough_collected_value(value),
    }
}

fn embedding_request_value(runtime: &Runtime, step: &EmbeddingRequestStep) -> Value {
    let bearer = TokenBearer {
        token: Some(SecretString::from(step.token.as_str())),
    };
    let input = EmbeddingRequestInput {
        model: &step.model,
        input: &step.input,
        api_base: step.api_base.as_deref(),
        env_base: step.env_base.as_deref(),
        credentials: None,
        caller_headers: &step.headers,
    };
    match runtime.block_on(embedding_request(&bearer, &input)) {
        Ok(request) => json!({
            "ok": true,
            "url": request.url,
            "headers": request.headers,
            "body": request.body,
        }),
        Err(error) => json!({"raised": error.status_code}),
    }
}

fn embedding_reply_value(step: &EmbeddingReplyStep) -> Value {
    match transform_response(step.body.as_bytes()) {
        Ok(reply) => json!({"ok": true, "value": embedding_value(&reply)}),
        Err(error) => json!({"raised": error.status_code}),
    }
}

fn passthrough_url_value(step: &PassthroughUrlStep) -> Value {
    let settings = config(step.env_base.clone(), None);
    let params = Object::new();
    let request = PassthroughUrlRequest {
        api_base: step.api_base.as_deref(),
        api_key: None,
        target: PassthroughTarget {
            model: "Embeddings",
            custom_llm_provider: "gigachat",
            endpoint: &step.endpoint,
        },
        request_query_params: None,
        litellm_params: &params,
    };
    match settings.get_complete_url(&request) {
        Ok((url, base)) => json!({"ok": true, "url": url.as_str(), "base": base}),
        Err(error) => error_status(&error),
    }
}

fn passthrough_headers_value(runtime: &Runtime, step: &PassthroughHeadersStep) -> Value {
    let bearer = TokenBearer {
        token: step
            .token
            .as_ref()
            .map(|token| SecretString::from(token.as_str())),
    };
    match runtime.block_on(validate_environment(step.headers.clone(), None, &bearer)) {
        Ok(headers) => json!({"ok": true, "headers": headers}),
        Err(error) => error_status(&error),
    }
}

fn passthrough_key_value(step: &PassthroughKeyStep) -> Value {
    let settings = config(None, step.env_key.clone());
    let api_key = step.api_key.clone().map(SecretString::from);
    settings.get_api_key(api_key.as_ref()).map_or_else(
        || json!({"ok": true, "key": null}),
        |secret| json!({"ok": true, "key": secret.expose_secret()}),
    )
}

fn config(base: Option<String>, key: Option<String>) -> GigachatPassthroughConfig {
    GigachatPassthroughConfig {
        api_base_env: base,
        api_key_env: key.map(SecretString::from),
    }
}

fn passthrough_streaming_value(step: &PassthroughStreamingStep) -> Value {
    let settings = GigachatPassthroughConfig::default();
    json!({
        "ok": true,
        "streaming": settings.is_streaming_request("", &step.request_data),
    })
}

fn passthrough_logging_value(step: &PassthroughLoggingStep) -> Value {
    let settings = GigachatPassthroughConfig::default();
    let target = PassthroughTarget {
        model: &step.model,
        custom_llm_provider: "gigachat",
        endpoint: &step.endpoint,
    };
    let response = RelayedResponse {
        status_code: step.status,
        body: step.body.as_bytes(),
    };
    let request_data = Object::new();
    match settings.logging_non_streaming_response(
        &target,
        &response,
        &request_data,
        &mut NoopLogging,
    ) {
        Some(GigachatPassthroughLogged::Chat(reply)) => {
            json!({"ok": true, "value": chat_value(&reply)})
        }
        Some(GigachatPassthroughLogged::Embedding(reply)) => {
            json!({"ok": true, "value": embedding_value(&reply)})
        }
        Some(GigachatPassthroughLogged::Chunks(_)) | None => json!({"ok": true, "value": null}),
    }
}

fn passthrough_collected_value(step: &PassthroughCollectedStep) -> Value {
    let settings = GigachatPassthroughConfig::default();
    let target = PassthroughTarget {
        model: &step.model,
        custom_llm_provider: "gigachat",
        endpoint: &step.endpoint,
    };
    match settings.handle_collected_chunks(&step.chunks, &mut NoopLogging, &target) {
        Some(GigachatPassthroughLogged::Chunks(chunks)) => {
            json!({"ok": true, "value": collected_value(&chunks)})
        }
        Some(GigachatPassthroughLogged::Chat(_) | GigachatPassthroughLogged::Embedding(_))
        | None => {
            json!({"ok": true, "value": null})
        }
    }
}
