import json
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict
from unittest.mock import patch

import httpx
import litellm
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.sagemaker.chat.transformation import SagemakerChatConfig
from litellm.llms.sagemaker.completion import transformation as completion
from litellm.llms.sagemaker.completion.transformation import SagemakerConfig
from litellm.llms.sagemaker.nova.transformation import SagemakerNovaConfig
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]

OBJECT: Final = TypeAdapter(Object)
FIXED_NOW: Final = 1_700_000_000
MODEL: Final = "endpoint"
COMPONENT_STATUS: Final = 418
CHAT_PROVIDER: Final = "sagemaker_chat"
NOVA_PROVIDER: Final = "sagemaker_nova"
UNREACHABLE: Final = {"status": "failure"}


class ParamsStep(TypedDict):
    kind: Literal["params"]
    non_default: NotRequired[Object]


class CompletionRequestStep(TypedDict):
    kind: Literal["completion_request"]
    model: str
    messages: NotRequired[list[AllMessageValues]]
    optional: NotRequired[Object]
    custom_prompt_dict: NotRequired[dict[str, Object]]
    global_custom_prompt_dict: NotRequired[dict[str, Object]]
    hf_model_name: NotRequired[str | None]
    now: NotRequired[int]


class CompletionResponseStep(TypedDict):
    kind: Literal["completion_response"]
    reply: JsonValue
    model: str
    prompt: NotRequired[str]
    created: NotRequired[int]


class ChatHeadersStep(TypedDict):
    kind: Literal["chat_headers"]
    headers: NotRequired[dict[str, str]]
    optional: NotRequired[Object]


class ChatPolicyStep(TypedDict):
    kind: Literal["chat_policy"]
    model: str
    chat_models: NotRequired[list[str]]


class ChatRequestStep(TypedDict):
    kind: Literal["chat_request", "nova_request"]
    model: str
    messages: NotRequired[list[AllMessageValues]]
    optional: NotRequired[Object]
    hf_model_name: NotRequired[str | None]


Step: TypeAlias = (
    ParamsStep
    | CompletionRequestStep
    | CompletionResponseStep
    | ChatHeadersStep
    | ChatPolicyStep
    | ChatRequestStep
)


class Input(TypedDict):
    steps: list[Step]


class RecordingLogging:
    def __init__(self) -> None:
        self.calls: list[Object] = []

    def post_call(self, **params: JsonValue) -> None:
        recorded: Object = {"input": params.get("input")}
        self.calls.append(recorded)


def unreachable(hf_model_name: str) -> Object:
    del hf_model_name
    return dict(UNREACHABLE)


def count_words(*, text: str, count_response_tokens: bool) -> int:
    del count_response_tokens
    return len(text.split())


def params_step(step: ParamsStep) -> Object:
    config = SagemakerConfig()
    non_default: Object = dict(step.get("non_default", {}))
    optional: Object = {}
    return {
        "kind": "params",
        "optional": OBJECT.validate_python(
            config.map_openai_params(
                non_default_params=dict(non_default),
                optional_params=dict(optional),
                model=MODEL,
                drop_params=False,
            )
        ),
        "remaining": OBJECT.validate_python(non_default),
    }


def completion_request_step(step: CompletionRequestStep) -> Object:
    config = SagemakerConfig()
    litellm_params: Object = {"custom_prompt_dict": step.get("custom_prompt_dict")}
    if step.get("hf_model_name") is not None:
        litellm_params["hf_model_name"] = step.get("hf_model_name")
    with (
        patch.object(litellm, "custom_prompt_dict", step.get("global_custom_prompt_dict") or {}),
        patch.object(huggingface_template_handler, "_get_tokenizer_config", unreachable),
        patch.object(huggingface_template_handler, "_get_chat_template_file", unreachable),
    ):
        projected = OBJECT.validate_python(
            config.transform_request(
                model=step["model"],
                messages=step.get("messages", []),
                optional_params=dict(step.get("optional", {})),
                litellm_params=dict(litellm_params),
                headers={},
            )
        )
    return {"kind": "completion_request", "keys": [*projected], "body": projected}


def completion_response_step(step: CompletionResponseStep) -> Object:
    config = SagemakerConfig()
    url = "https://runtime.sagemaker.test/endpoints/endpoint/invocations"
    raw = httpx.Response(
        200,
        content=json.dumps(step["reply"]),
        request=httpx.Request("POST", url),
    )
    logging = RecordingLogging()
    with (
        patch.object(completion.time, "time", return_value=step.get("created", FIXED_NOW)),
        patch.object(completion, "token_counter", count_words),
    ):
        answered = config.transform_response(
            model=step["model"],
            raw_response=raw,
            model_response=litellm.ModelResponse(),
            logging_obj=logging,
            request_data={"inputs": step.get("prompt", "")},
            messages=[],
            optional_params={},
            litellm_params={},
            encoding=None,
        )
    choice = answered.choices[0]
    usage = answered.usage
    return {
        "kind": "completion_response",
        "model": answered.model,
        "created": answered.created,
        "content": choice.message.content,
        "finish_reason": choice.finish_reason,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }


def chat_headers_step(step: ChatHeadersStep) -> Object:
    config = SagemakerChatConfig()
    merged = config.validate_environment(
        headers=dict(step.get("headers", {})),
        model=MODEL,
        messages=[],
        optional_params=dict(step.get("optional", {})),
        litellm_params={},
    )
    failure = config.get_error_class(
        error_message="boom",
        status_code=COMPONENT_STATUS,
        headers={"x-fixture": "kept"},
    )
    return {
        "kind": "chat_headers",
        "headers": OBJECT.validate_python(merged),
        "error": {
            "status_code": failure.status_code,
            "headers": OBJECT.validate_python(failure.headers or {}),
        },
    }


def chat_policy_step(step: ChatPolicyStep) -> Object:
    chat = SagemakerChatConfig()
    nova = SagemakerNovaConfig()
    catalog = set(step.get("chat_models", []))
    with patch.object(litellm, "open_ai_chat_completion_models", catalog):
        chat_supported: list[JsonValue] = list(
            chat.get_supported_openai_params(model=step["model"])
        )
        nova_supported: list[JsonValue] = list(
            nova.get_supported_openai_params(model=step["model"])
        )
    return {
        "kind": "chat_policy",
        "chat_supported": chat_supported,
        "nova_supported": nova_supported,
        "chat_flags": [chat.has_custom_stream_wrapper, chat.supports_stream_param_in_request_body],
        "nova_flags": [nova.has_custom_stream_wrapper, nova.supports_stream_param_in_request_body],
    }


def chat_request_step(step: ChatRequestStep) -> Object:
    if step["kind"] == "nova_request":
        config = SagemakerNovaConfig()
        provider = NOVA_PROVIDER
    else:
        config = SagemakerChatConfig()
        provider = CHAT_PROVIDER
    litellm_params: Object = {"custom_llm_provider": provider}
    if step.get("hf_model_name") is not None:
        litellm_params["hf_model_name"] = step.get("hf_model_name")
    projected = OBJECT.validate_python(
        config.transform_request(
            model=step["model"],
            messages=step.get("messages", []),
            optional_params=dict(step.get("optional", {})),
            litellm_params=dict(litellm_params),
            headers={},
        )
    )
    return {"kind": step["kind"], "keys": [*projected], "body": projected}


def run(input_value: Input) -> list[Object]:
    results: list[Object] = []
    for step in input_value["steps"]:
        if step["kind"] == "params":
            results.append(params_step(step))
        elif step["kind"] == "completion_request":
            results.append(completion_request_step(step))
        elif step["kind"] == "completion_response":
            results.append(completion_response_step(step))
        elif step["kind"] == "chat_headers":
            results.append(chat_headers_step(step))
        elif step["kind"] == "chat_policy":
            results.append(chat_policy_step(step))
        else:
            results.append(chat_request_step(step))
    return results
