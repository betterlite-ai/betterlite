import os
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.fal_ai.chat.transformation import FalAIChatConfig, FalAIError
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

HEADERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
EMPTY: dict[str, JsonValue] = {}
NO_HEADERS: dict[str, str] = {}
NO_MESSAGES: list[dict[str, JsonValue]] = []
VALID_MESSAGES: list[dict[str, JsonValue]] = [
    {
        "role": "user",
        "content": [
            {"type": "text", "text": "t"},
            {"type": "image_url", "image_url": {"url": "u"}},
        ],
    }
]
MODEL = "fal_ai/model"


class ChatConfig(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        _headers: dict[str, str],
        _model: str,
        _messages: list[AllMessageValues],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _api_key: str | None = None,
        _api_base: str | None = None,
        /,
    ) -> dict[str, str]: ...  # positional-only: LiteLLM names them api_key and api_base

    def transform_request(
        self,
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...


class NoChatMessages(list[AllMessageValues]):
    pass


class RequestCase(TypedDict):
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]


class RequestsCall(TypedDict):
    operation: Literal["one_user_required", "joined_text", "empty_image_url"]
    cases: list[RequestCase]


class MapCase(TypedDict):
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]
    drop_params: bool


class ReasoningOverride(TypedDict):
    operation: Literal["reasoning_override"]
    model: str
    cases: list[MapCase]


class HeaderCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    api_base: str | None
    model: str


class HeaderOverlay(TypedDict):
    operation: Literal["header_overlay"]
    environment: dict[str, str]
    cases: list[HeaderCase]


class ReplyCase(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class RepliesCall(TypedDict):
    model: str
    replies: list[ReplyCase]


class MalformedReply(RepliesCall):
    operation: Literal["malformed_reply"]


class ReasoningUsage(RepliesCall):
    operation: Literal["reasoning_usage"]


Input: TypeAlias = (
    RequestsCall | ReasoningOverride | HeaderOverlay | MalformedReply | ReasoningUsage
)


class MessageDump(TypedDict):
    role: str
    content: str | None
    reasoning_content: NotRequired[str | None]


class ChoiceDump(TypedDict):
    finish_reason: str
    message: MessageDump


class UsageDump(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseDump(TypedDict):
    id: str
    object: str
    created: int
    model: str
    choices: list[ChoiceDump]
    usage: UsageDump


RESPONSE: TypeAdapter[ResponseDump] = TypeAdapter(ResponseDump)


def apply_environment(environment: dict[str, str]) -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def request_outcome(
    config: ChatConfig,
    messages: list[dict[str, JsonValue]],
    optional: dict[str, JsonValue],
) -> JsonValue:
    try:
        body = config.transform_request(
            MODEL,
            cast("list[AllMessageValues]", messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            dict(optional),
            EMPTY,
            NO_HEADERS,
        )
    except FalAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/chat/transformation.py#L52
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "value": body}


def mapping_outcomes(config: ChatConfig, call: ReasoningOverride) -> JsonValue:
    outcomes: list[JsonValue] = []
    for case in call["cases"]:
        try:
            mapped = config.map_openai_params(
                dict(case["non_default"]),
                dict(case["optional"]),
                call["model"],
                drop_params=case["drop_params"],
            )
        except FalAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/chat/transformation.py#L52
            outcomes.append({"mapped": {"error": {"status": error.status_code}}})
            continue
        request = request_outcome(config, VALID_MESSAGES, mapped)
        outcomes.append({"mapped": {"params": mapped}, "request": request})
    return {"cases": outcomes}


def header_outcomes(config: ChatConfig, call: HeaderOverlay) -> JsonValue:
    apply_environment(call["environment"])
    outcomes: list[JsonValue] = []
    for case in call["cases"]:
        url = config.get_complete_url(case["api_base"], None, case["model"], EMPTY, EMPTY)
        try:
            headers = config.validate_environment(
                dict(case["headers"]),
                case["model"],
                NoChatMessages(),
                EMPTY,
                EMPTY,
                case["api_key"],
                None,
            )
        except ValueError:
            outcomes.append({"url": url, "error": {"status": None}})
        else:
            outcomes.append({"url": url, "headers": HEADERS.validate_python(headers)})
    return {"cases": outcomes}


def reply_outcome(config: FalAIChatConfig, model: str, case: ReplyCase) -> JsonValue:
    response = httpx.Response(
        case["status"],
        headers=case["headers"],
        stream=httpx.ByteStream(case["body"].encode()),
        request=httpx.Request("POST", "https://host"),
    )
    response.read()
    try:
        result = config.transform_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/chat/transformation.py#L195-L233
            model=model,
            raw_response=response,
            model_response=ModelResponse(id="chatcmpl-1", created=1, model="old-model"),
            logging_obj=None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/chat/transformation.py#L200
            request_data=EMPTY,
            messages=cast("list[AllMessageValues]", NO_MESSAGES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=EMPTY,
            litellm_params=EMPTY,
            encoding=None,
        )
    except FalAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/chat/transformation.py#L52
        return {
            "ok": False,
            "status": error.status_code,
            "headers": HEADERS.validate_python(error.headers),  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L46
            "message": error.message,
        }
    dumped = RESPONSE.validate_json(result.model_dump_json())
    choice = dumped["choices"][0]
    usage = dumped["usage"]
    return {
        "ok": True,
        "value": {
            "id": dumped["id"],
            "object": dumped["object"],
            "created_stamped": dumped["created"] != 1,
            "model": dumped["model"],
            "role": choice["message"]["role"],
            "content": choice["message"]["content"],
            "reasoning_content": choice["message"].get("reasoning_content"),
            "finish_reason": choice["finish_reason"],
            "usage": {
                "prompt_tokens": usage["prompt_tokens"],
                "completion_tokens": usage["completion_tokens"],
                "total_tokens": usage["total_tokens"],
            },
        },
    }


def run(input_value: Input) -> JsonValue:
    config = FalAIChatConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L69
    if input_value["operation"] == "reasoning_override":
        return mapping_outcomes(config, input_value)
    if input_value["operation"] == "header_overlay":
        return header_outcomes(config, input_value)
    if input_value["operation"] == "malformed_reply":
        replies = input_value["replies"]
        return {"cases": [reply_outcome(config, input_value["model"], case) for case in replies]}
    if input_value["operation"] == "reasoning_usage":
        replies = input_value["replies"]
        return {"cases": [reply_outcome(config, input_value["model"], case) for case in replies]}
    return {
        "cases": [
            request_outcome(config, case["messages"], case["optional_params"])
            for case in input_value["cases"]
        ]
    }
