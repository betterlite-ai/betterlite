import json
from typing import Final, Literal, TypeAlias, TypedDict

import httpx
import litellm
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .boundary import (
    CHAT,
    COMPLETION,
    COMPONENT_STATUS,
    DROP_PARAMS,
    LOGGING,
    MODEL,
    NOVA,
    ChatConfig,
    Object,
)

CHAT_PROVIDER: Final = "sagemaker_chat"
NOVA_PROVIDER: Final = "sagemaker_nova"


class ParamsStep(TypedDict):
    kind: Literal["params"]
    non_default: Object


class CompletionRequestStep(TypedDict):
    kind: Literal["completion_request"]
    model: str
    messages: list[JsonValue]
    optional: Object
    custom_prompt_dict: Object
    global_custom_prompt_dict: Object
    hf_model_name: str | None
    now: int


class CompletionResponseStep(TypedDict):
    kind: Literal["completion_response"]
    reply: JsonValue
    model: str
    prompt: str
    created: int


class ChatHeadersStep(TypedDict):
    kind: Literal["chat_headers"]
    headers: dict[str, str]
    optional: Object


class ChatPolicyStep(TypedDict):
    kind: Literal["chat_policy"]
    model: str
    chat_models: list[str]


class ChatRequestStep(TypedDict):
    kind: Literal["chat_request", "nova_request"]
    model: str
    messages: list[JsonValue]
    optional: Object
    hf_model_name: str | None


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


class Usage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ReplyMessage(TypedDict):
    content: str | None


class ReplyChoice(TypedDict):
    message: ReplyMessage
    finish_reason: str | None


class Reply(TypedDict):
    model: str | None
    created: int
    choices: list[ReplyChoice]
    usage: Usage


INPUT: Final = TypeAdapter(Input)
REPLY: Final = TypeAdapter(Reply)


def params_step(step: ParamsStep) -> Object:
    optional: Object = {}
    COMPLETION.map_openai_params(
        step["non_default"], optional, MODEL, drop_params=DROP_PARAMS  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L79-L112
    )
    return {"kind": "params", "optional": optional, "remaining": step["non_default"]}


def completion_request_step(step: CompletionRequestStep) -> Object:
    prompt_dict = step["custom_prompt_dict"] or step["global_custom_prompt_dict"]
    litellm_params: Object = {
        "custom_prompt_dict": prompt_dict,
        "hf_model_name": step["hf_model_name"],
    }
    body = COMPLETION.transform_request(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L165-L195
        step["model"], step["messages"], step["optional"], litellm_params, {}
    )
    return {"kind": "completion_request", "keys": [*body], "body": body}


def completion_response_step(step: CompletionResponseStep) -> Object:
    raw = httpx.Response(
        200,
        content=json.dumps(step["reply"]),
        request=httpx.Request("POST", "https://runtime.sagemaker.test/endpoint"),
    )
    model_response = ModelResponse()
    COMPLETION.transform_response(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/completion/transformation.py#L197-L258
        step["model"],
        raw,
        model_response,
        LOGGING,
        {"inputs": step["prompt"]},
        [],
        {},
        {},
        None,
    )
    reply = REPLY.validate_json(model_response.model_dump_json())
    choice = reply["choices"][0]
    usage = reply["usage"]
    return {
        "kind": "completion_response",
        "model": reply["model"],
        "created": reply["created"],
        "content": choice["message"]["content"],
        "finish_reason": choice["finish_reason"],
        "usage": {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
        },
    }


def chat_headers_step(step: ChatHeadersStep) -> Object:
    merged = CHAT.validate_environment(step["headers"], MODEL, [], step["optional"], {})
    failure = CHAT.get_error_class(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L33-L47
        "boom", COMPONENT_STATUS, {"x-fixture": "kept"}
    )
    return {
        "kind": "chat_headers",
        "headers": merged,
        "error": {"status_code": failure.status_code},
    }


def chat_policy_step(step: ChatPolicyStep) -> Object:
    litellm.open_ai_chat_completion_models = set(step["chat_models"])
    chat_supported: list[JsonValue] = [*CHAT.get_supported_openai_params(step["model"])]
    nova_supported: list[JsonValue] = [*NOVA.get_supported_openai_params(step["model"])]
    chat_flags: list[JsonValue] = [
        CHAT.has_custom_stream_wrapper,
        CHAT.supports_stream_param_in_request_body,
    ]
    nova_flags: list[JsonValue] = [
        NOVA.has_custom_stream_wrapper,
        NOVA.supports_stream_param_in_request_body,
    ]
    return {
        "kind": "chat_policy",
        "chat_supported": chat_supported,
        "nova_supported": nova_supported,
        "chat_flags": chat_flags,
        "nova_flags": nova_flags,
    }


def chat_request_step(step: ChatRequestStep) -> Object:
    config: ChatConfig
    provider: str
    if step["kind"] == "nova_request":
        config = NOVA
        provider = NOVA_PROVIDER
    else:
        config = CHAT
        provider = CHAT_PROVIDER
    litellm_params: Object = {
        "custom_llm_provider": provider,
        "hf_model_name": step["hf_model_name"],
    }
    body = config.transform_request(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L64-L82
        step["model"], step["messages"], step["optional"], litellm_params, {}
    )
    return {"kind": step["kind"], "keys": [*body], "body": body}


def run(input_value: Input) -> list[Object]:
    results: list[Object] = []
    for step in INPUT.validate_python(input_value)["steps"]:
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
