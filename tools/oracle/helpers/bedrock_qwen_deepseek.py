import time
from functools import partial
from typing import Final, Literal, Protocol, TypedDict, Unpack, cast

import httpx
import litellm
from litellm.llms.bedrock.chat.invoke_transformations.amazon_deepseek_transformation import (
    AmazonDeepSeekR1Config,
)
from litellm.llms.bedrock.chat.invoke_transformations.amazon_qwen2_transformation import (
    AmazonQwen2Config,
)
from litellm.llms.bedrock.chat.invoke_transformations.amazon_qwen3_transformation import (
    AmazonQwen3Config,
)
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import Choices, Message, ModelResponse, Usage
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# register_model prints the provider list to stdout
litellm.suppress_debug_info = True


class Logging(Protocol):
    def post_call(self, **kwargs: JsonValue) -> None: ...


class Sink(Logging):
    def __init__(self) -> None:
        self.kwargs: dict[str, JsonValue] = {}

    @override
    def post_call(self, **kwargs: JsonValue) -> None:
        self.kwargs = kwargs


class TransformResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Logging
    request_data: dict[str, JsonValue]
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    encoding: None
    api_key: None
    json_mode: None


# The three Bedrock invoke configs share the base method signature, which the typed dict mirrors.
class ReplyApi(Protocol):
    def transform_response(self, **kwargs: Unpack[TransformResponseArgs]) -> ModelResponse: ...


class QwenRequestApi(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...


Role = Literal["assistant", "user", "system", "tool", "function"]
Projection = Literal["qwen2", "qwen3"]


class MessageSeed(TypedDict):
    role: Role
    content: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class ChoiceSeed(TypedDict):
    index: int
    finish_reason: str | None
    message: MessageSeed


class UsageSeed(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseSeed(TypedDict):
    id: str
    created: int
    model: str
    choices: list[ChoiceSeed]
    usage: UsageSeed


class QwenReplyCall(TypedDict):
    projection: Projection
    model: str
    messages: list[JsonValue]
    body: str
    response: ResponseSeed


class QwenRequestCall(TypedDict):
    model: str
    messages: list[JsonValue]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class DeepseekReplyCall(TypedDict):
    model: str
    status: int
    headers: list[list[str]]
    request_data: dict[str, JsonValue]
    body: str
    response: ResponseSeed


class Input(TypedDict):
    qwen_reply: list[QwenReplyCall]
    qwen_request: list[QwenRequestCall]
    deepseek_reply: list[DeepseekReplyCall]


# The model response carries the seed's shape plus the object name the constructor fixes.
class ResponseView(ResponseSeed):
    object: str


RESPONSE: Final[TypeAdapter[ResponseView]] = TypeAdapter(ResponseView)


def build_model_response(seed: ResponseSeed) -> ModelResponse:
    choices: list[Choices] = [
        Choices(
            index=choice["index"],
            message=Message(
                content=choice["message"]["content"],
                role=choice["message"]["role"],
                provider_specific_fields=choice["message"]["provider_specific_fields"],
            ),
            finish_reason=choice["finish_reason"],
        )
        for choice in seed["choices"]
    ]
    return ModelResponse(
        id=seed["id"],
        choices=choices,
        created=seed["created"],
        model=seed["model"],
        usage=Usage(**seed["usage"]),
    )


def reply_view(transformed: ModelResponse) -> JsonValue:
    view = RESPONSE.validate_json(transformed.model_dump_json())
    choices: list[JsonValue] = []
    for choice in view["choices"]:
        fields = choice["message"]["provider_specific_fields"]
        choices.append(
            {
                "index": choice["index"],
                "finish_reason": choice["finish_reason"],
                "content": choice["message"]["content"],
                "reasoning_content": None if fields is None else fields.get("reasoning_content"),
                "provider_specific_fields": fields,
            }
        )
    carried_usage = view["usage"]
    usage: JsonValue = {
        "prompt_tokens": carried_usage["prompt_tokens"],
        "completion_tokens": carried_usage["completion_tokens"],
        "total_tokens": carried_usage["total_tokens"],
    }
    outcome: JsonValue = {
        "id": view["id"],
        "object": view["object"],
        "created": view["created"],
        "model": view["model"],
        "usage": usage,
        "choices": choices,
    }
    return outcome


def qwen_reply_config(projection: Projection) -> ReplyApi:
    if projection == "qwen2":
        return cast("ReplyApi", AmazonQwen2Config())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_qwen2_transformation.py#L27-L35
    return cast("ReplyApi", AmazonQwen3Config())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_qwen3_transformation.py#L25-L46


def deepseek_config() -> ReplyApi:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
        "ReplyApi",
        AmazonDeepSeekR1Config(),  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
    )


def qwen_reply_result(call: QwenReplyCall) -> JsonValue:
    config = qwen_reply_config(call["projection"])
    sink = Sink()
    raw_response = httpx.Response(200, content=call["body"].encode(), headers=[])
    try:
        transformed = config.transform_response(
            model=call["model"], raw_response=raw_response,
            model_response=build_model_response(call["response"]), logging_obj=sink,
            request_data={}, messages=call["messages"], optional_params={},
            litellm_params={}, encoding=None, api_key=None, json_mode=None,
        )
    except Exception as failure:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_qwen3_transformation.py#L211-L219
        return {
            "error": {"class": type(failure).__name__, "status": None},
            "logged": bool(sink.kwargs),
        }
    return {"ok": reply_view(transformed), "logged": bool(sink.kwargs)}


def qwen_request_result(config: QwenRequestApi, call: QwenRequestCall) -> JsonValue:
    try:
        mapped = config.map_openai_params(
            call["non_default_params"],
            dict(call["optional_params"]),
            call["model"],
            drop_params=False,
        )
        body = config.transform_request(call["model"], call["messages"], mapped, {}, {})
    except Exception as failure:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_qwen3_transformation.py#L117-L161
        return {"error": {"class": type(failure).__name__, "status": None}}
    return {
        "ok": {
            "supported": cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_qwen3_transformation.py#L52-L60
                JsonValue, config.get_supported_openai_params(call["model"])
            ),
            "mapped": mapped,
            "body": body,
        }
    }


def deepseek_reply_result(config: ReplyApi, call: DeepseekReplyCall) -> JsonValue:
    raw_response = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=[(pair[0], pair[1]) for pair in call["headers"]],
    )
    saved_time = time.time
    time.time = partial(float, 0)
    try:
        transformed = config.transform_response(
            model=call["model"], raw_response=raw_response,
            model_response=build_model_response(call["response"]), logging_obj=Sink(),
            request_data=dict(call["request_data"]), messages=[], optional_params={},
            litellm_params={}, encoding=None, api_key=None, json_mode=None,
        )
    except BedrockError as error:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L295-L302
        return {"error": {"class": type(error).__name__, "status": error.status_code}}
    except Exception as failure:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_deepseek_transformation.py#L32-L79
        return {"error": {"class": type(failure).__name__, "status": None}}
    finally:
        time.time = saved_time
    return {"ok": reply_view(transformed)}


def run(argument: Input) -> JsonValue:
    request_config = cast("QwenRequestApi", AmazonQwen3Config())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/amazon_qwen3_transformation.py#L52-L82
    results: list[JsonValue] = [
        *[qwen_reply_result(call) for call in argument["qwen_reply"]],
        *[qwen_request_result(request_config, call) for call in argument["qwen_request"]],
        *[deepseek_reply_result(deepseek_config(), call) for call in argument["deepseek_reply"]],
    ]
    answer: JsonValue = {"results": results}
    return answer
