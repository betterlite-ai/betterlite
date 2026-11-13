import datetime as dt
import json
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict
from unittest.mock import patch

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.sagemaker.chat.transformation import SagemakerChatConfig
from litellm.llms.sagemaker.completion import transformation as completion
from litellm.llms.sagemaker.completion.transformation import SagemakerConfig
from litellm.llms.sagemaker.nova.transformation import SagemakerNovaConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]

OBJECT: Final = TypeAdapter(Object)
FIXED_NOW: Final = 1_700_000_000
MODEL: Final = "endpoint"
COMPONENT_STATUS: Final = 418
CHAT_PROVIDER: Final = "sagemaker_chat"
NOVA_PROVIDER: Final = "sagemaker_nova"
DROP_PARAMS: Final = False
UNREACHABLE: Final = {"status": "failure"}
LOG_START: Final = dt.datetime.fromtimestamp(FIXED_NOW, dt.UTC)


class ParamsStep(TypedDict):
    kind: Literal["params"]
    non_default: NotRequired[Object]


class CompletionRequestStep(TypedDict):
    kind: Literal["completion_request"]
    model: str
    messages: NotRequired[list[AllMessageValues]]
    optional: NotRequired[Object]
    custom_prompt_dict: NotRequired[JsonValue]
    global_custom_prompt_dict: NotRequired[JsonValue]
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


def unreachable(hf_model_name: str) -> Object:
    del hf_model_name
    return dict(UNREACHABLE)


def count_words(*, text: str, count_response_tokens: bool) -> int:
    del count_response_tokens
    return len(text.split())


def as_object(value: JsonValue) -> Object:
    return OBJECT.validate_json(json.dumps(value))


def params_step(step: ParamsStep) -> Object:
    config = SagemakerConfig()
    non_default: Object = dict(step.get("non_default", {}))
    optional: Object = {}
    return {
        "kind": "params",
        "optional": as_object(
            json.dumps(
                config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L79-L112
                    dict(non_default), optional, MODEL, DROP_PARAMS
                )
            )
        ),
        "remaining": as_object(non_default),
    }


def completion_request_step(step: CompletionRequestStep) -> Object:
    config = SagemakerConfig()
    litellm_params: Object = {
        "custom_prompt_dict": step.get("custom_prompt_dict"),
        "hf_model_name": step.get("hf_model_name"),
    }
    with (
        patch.object(litellm, "custom_prompt_dict", step.get("global_custom_prompt_dict") or {}),
        patch.object(huggingface_template_handler, "_get_tokenizer_config", unreachable),
        patch.object(huggingface_template_handler, "_get_chat_template_file", unreachable),
    ):
        body = as_object(
            json.dumps(
                config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L165-L195
                    step["model"],
                    step.get("messages", []),
                    dict(step.get("optional", {})),
                    litellm_params,
                    {},
                )
            )
        )
    return {"kind": "completion_request", "keys": [*body], "body": body}


def completion_response_step(step: CompletionResponseStep) -> Object:
    config = SagemakerConfig()
    url = "https://runtime.sagemaker.test/endpoints/endpoint/invocations"
    raw = httpx.Response(
        200,
        content=json.dumps(step["reply"]),
        request=httpx.Request("POST", url),
    )
    model_response = ModelResponse()
    with (
        patch.object(completion.time, "time", return_value=step.get("created", FIXED_NOW)),
        patch.object(completion, "token_counter", count_words),
    ):
        config.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L197-L258
            step["model"],
            raw,
            model_response,
            Logging(
                model=MODEL,
                messages=None,
                stream=False,
                call_type="text_completion",
                start_time=LOG_START,
                litellm_call_id="call-1",
                function_id="fn-1",
            ),
            {"inputs": step.get("prompt", "")},
            [],
            {},
            {},
            None,
        )
    choice = model_response.choices[0]
    return {
        "kind": "completion_response",
        "model": model_response.model,
        "created": model_response.created,
        "content": choice.message.content,
        "finish_reason": choice.finish_reason,
    }


def chat_headers_step(step: ChatHeadersStep) -> Object:
    config = SagemakerChatConfig()
    failure = config.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L46-L47
        "boom", COMPONENT_STATUS, {"x-fixture": "kept"}
    )
    return {
        "kind": "chat_headers",
        "headers": as_object(
            json.dumps(
                config.validate_environment(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L49-L62
                    dict(step.get("headers", {})),
                    MODEL,
                    [],
                    dict(step.get("optional", {})),
                    {},
                )
            )
        ),
        "error": {"status_code": failure.status_code},
    }


def chat_policy_step(step: ChatPolicyStep) -> Object:
    chat = SagemakerChatConfig()
    nova = SagemakerNovaConfig()
    catalog = set(step.get("chat_models", []))
    with patch.object(litellm, "open_ai_chat_completion_models", catalog):
        chat_supported = as_object(
            json.dumps(
                {
                    "params": chat.get_supported_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/nova/transformation.py#L34-L46
                        step["model"]
                    )
                }
            )
        )
        nova_supported = as_object(
            json.dumps(
                {
                    "params": nova.get_supported_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/nova/transformation.py#L34-L46
                        step["model"]
                    )
                }
            )
        )
    return {
        "kind": "chat_policy",
        "chat_supported": chat_supported["params"],
        "nova_supported": nova_supported["params"],
        "chat_flags": [chat.has_custom_stream_wrapper, chat.supports_stream_param_in_request_body],
        "nova_flags": [nova.has_custom_stream_wrapper, nova.supports_stream_param_in_request_body],
    }


def chat_request_step(step: ChatRequestStep) -> Object:
    if step["kind"] == "nova_request":
        provider = NOVA_PROVIDER
        chat = SagemakerNovaConfig()
    else:
        provider = CHAT_PROVIDER
        chat = SagemakerChatConfig()
    litellm_params: Object = {
        "custom_llm_provider": provider,
        "hf_model_name": step.get("hf_model_name"),
    }
    body = as_object(
        json.dumps(
            chat.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L64-L82
                step["model"],
                step.get("messages", []),
                dict(step.get("optional", {})),
                litellm_params,
                {},
            )
        )
    )
    return {"kind": step["kind"], "keys": [*body], "body": body}


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
