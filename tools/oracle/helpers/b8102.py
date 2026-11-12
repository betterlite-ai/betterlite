import os
from typing import Literal, NotRequired, TypedDict, cast

import httpx
import litellm.utils
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.langgraph.chat.transformation import LangGraphConfig, LangGraphError
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import Choices, ModelResponse, Usage
from pydantic import JsonValue

COUNT_MODEL = "gpt-3.5-turbo"
BASE_ENV = "LANGGRAPH_API_BASE"
KEY_ENV = "LANGGRAPH_API_KEY"


class TextPart(TypedDict):
    type: Literal["text"]
    text: str


class Message(TypedDict):
    role: str
    content: str | list[TextPart]
    metadata: NotRequired[dict[str, JsonValue]]


class PromptMessage(TypedDict):
    role: str
    content: str


class InfoCase(TypedDict):
    api_base: str | None
    api_key: str | None
    env_api_base: str | None
    env_api_key: str | None


class UrlCase(TypedDict):
    api_base: str | None
    stream: bool


class HeadersCase(TypedDict):
    api_key: str | None
    headers: dict[str, str]


class RequestCase(TypedDict):
    model: str
    messages: list[Message]
    options: dict[str, JsonValue]
    stream: bool


class ReplyCase(TypedDict):
    model: str
    body: str
    status: int


class InfoInput(TypedDict):
    op: Literal["info"]
    cases: list[InfoCase]


class UrlInput(TypedDict):
    op: Literal["url"]
    cases: list[UrlCase]


class HeadersInput(TypedDict):
    op: Literal["headers"]
    cases: list[HeadersCase]


class RequestInput(TypedDict):
    op: Literal["request"]
    cases: list[RequestCase]


class ReplyInput(TypedDict):
    op: Literal["reply"]
    cases: list[ReplyCase]


Input = InfoInput | UrlInput | HeadersInput | RequestInput | ReplyInput

PROMPT: list[PromptMessage] = [{"role": "user", "content": "hello there"}]
EMPTY: dict[str, JsonValue] = {}
NO_MESSAGES: list[AllMessageValues] = []


def fake_token_counter(
    *,
    model: str,
    messages: list[PromptMessage] | None = None,
    text: str | None = None,
    count_response_tokens: bool = False,
) -> int:
    if model != COUNT_MODEL:
        raise ValueError(model)
    if messages is not None:
        return sum(len(message["content"].split()) for message in messages)
    if text is None or not count_response_tokens:
        raise ValueError("no reply text to count")
    return len(text.split())


def provider_info(case: InfoCase) -> JsonValue:
    os.environ.pop(BASE_ENV, None)
    os.environ.pop(KEY_ENV, None)
    if case["env_api_base"] is not None:
        os.environ[BASE_ENV] = case["env_api_base"]
    if case["env_api_key"] is not None:
        os.environ[KEY_ENV] = case["env_api_key"]
    _, _, key, base = get_llm_provider(
        model="langgraph/agent", api_base=case["api_base"], api_key=case["api_key"]
    )
    os.environ.pop(BASE_ENV, None)
    os.environ.pop(KEY_ENV, None)
    return {"base": base, "key": key}


def info(input_value: InfoInput) -> JsonValue:
    config = LangGraphConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L52
    supported: list[JsonValue] = list(config.get_supported_openai_params(model="agent"))
    return {
        "supported": supported,
        "body_stream": config.supports_stream_param_in_request_body,
        "fake_stream": config.should_fake_stream(model="agent", stream=True),
        "cases": [provider_info(case) for case in input_value["cases"]],
    }


def url_case(config: LangGraphConfig, case: UrlCase) -> JsonValue:
    try:
        complete = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L92
            api_base=case["api_base"],
            api_key=None,
            model="agent",
            optional_params=EMPTY,
            litellm_params=EMPTY,
            stream=case["stream"],
        )
    except ValueError:
        return {"ok": False}
    return {"ok": True, "url": complete}


def url(input_value: UrlInput) -> JsonValue:
    config = LangGraphConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L52
    return {"urls": [url_case(config, case) for case in input_value["cases"]]}


def headers_case(config: LangGraphConfig, case: HeadersCase) -> JsonValue:
    return config.validate_environment(  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L471
        headers=case["headers"],
        model="agent",
        messages=NO_MESSAGES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=EMPTY,
        litellm_params=EMPTY,
        api_key=case["api_key"],
    )


def headers(input_value: HeadersInput) -> JsonValue:
    config = LangGraphConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L52
    return {"headers": [headers_case(config, case) for case in input_value["cases"]]}


def request_case(config: LangGraphConfig, case: RequestCase) -> JsonValue:
    params: dict[str, JsonValue] = {"stream": case["stream"]}
    return config.transform_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L182
        model=case["model"],
        messages=cast("list[AllMessageValues]", case["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=case["options"],
        litellm_params=params,
        headers=EMPTY,
    )


def request(input_value: RequestInput) -> JsonValue:
    config = LangGraphConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L52
    return {"bodies": [request_case(config, case) for case in input_value["cases"]]}


def reply_view(result: ModelResponse) -> JsonValue:
    choice = result.choices[0]
    usage: Usage | None = result.get("usage")  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2264
    match choice:
        case Choices(message=message):  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1644
            return {
                "model": result.model,
                "finish_reason": choice.finish_reason,
                "role": message.role,
                "content": message.content,
                "usage": None
                if usage is None
                else [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
            }
    raise TypeError("a LangGraph reply holds one non-streaming choice")


def raw_reply(case: ReplyCase) -> httpx.Response:
    return httpx.Response(
        case["status"],
        content=case["body"].encode(),
        request=httpx.Request("POST", "https://host"),
    )


def transform_reply(config: LangGraphConfig, case: ReplyCase) -> ModelResponse:
    return config.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L412
        model=case["model"],
        raw_response=raw_reply(case),
        model_response=ModelResponse(),
        logging_obj=None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L32
        request_data=EMPTY,
        messages=cast("list[AllMessageValues]", PROMPT),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=EMPTY,
        litellm_params=EMPTY,
        encoding=None,
    )


def reply_case(config: LangGraphConfig, case: ReplyCase) -> JsonValue:
    try:
        result = transform_reply(config, case)
    except LangGraphError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L40
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "reply": reply_view(result)}


def reply(input_value: ReplyInput) -> JsonValue:
    config = LangGraphConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langgraph/chat/transformation.py#L52
    litellm.utils.token_counter = fake_token_counter  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L2550
    return {"replies": [reply_case(config, case) for case in input_value["cases"]]}


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "info":
        return info(input_value)
    if input_value["op"] == "url":
        return url(input_value)
    if input_value["op"] == "headers":
        return headers(input_value)
    if input_value["op"] == "request":
        return request(input_value)
    return reply(input_value)
