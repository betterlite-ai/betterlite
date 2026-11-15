import os
from collections.abc import Awaitable, Callable
from typing import NotRequired, Protocol, TypedDict
from urllib.parse import parse_qsl

import httpx
import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.petals.completion.transformation import PetalsConfig
from litellm.llms.vllm.common_utils import VLLMModelInfo
from litellm.llms.vllm.passthrough.transformation import VLLMPassthroughConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse, Usage
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True


HeaderMap = dict[str, str]
JsonMap = dict[str, JsonValue]


class Config(Protocol):
    validate_environment: Callable[
        [HeaderMap, str, list[AllMessageValues], JsonMap, JsonMap, str | None],
        dict[str, JsonValue],
    ]

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, JsonValue]
    ) -> BaseLLMException: ...


class Petals(Config, Protocol):
    @classmethod
    def get_config(cls) -> dict[str, JsonValue]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class Passthrough(Protocol):
    def is_streaming_request(self, endpoint: str, request_data: dict[str, JsonValue]) -> bool: ...

    get_complete_url: Callable[
        [str | None, str | None, str, str, JsonMap | None, JsonMap],
        tuple[httpx.URL, str],
    ]


INFO_VIEW: Config = VLLMModelInfo()
PASS_VIEW: Passthrough = VLLMPassthroughConfig()
PETALS_VIEW: Petals = PetalsConfig()


class InfoCall(TypedDict):
    model: str
    api_base: str | None
    env: dict[str, str]
    headers: dict[str, str]
    api_key: NotRequired[str | None]


class PassCall(TypedDict):
    endpoint: str
    api_base: str | None
    env: dict[str, str]
    query: dict[str, JsonValue] | None
    request_data: dict[str, JsonValue]


class MapCase(TypedDict):
    non_default: dict[str, JsonValue]
    optional: NotRequired[dict[str, JsonValue]]


class ValidateCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None


class RequestCase(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str | None
    max_tokens: NotRequired[int | None]
    temperature: NotRequired[float | None]
    top_p: NotRequired[float | None]
    stream: NotRequired[bool | None]


class ReplyCase(TypedDict):
    model: str
    body: str
    status: int
    headers: dict[str, str]
    messages: list[dict[str, JsonValue]]


class ReplyDump(TypedDict):
    usage: Usage


class Responder:
    def __init__(
        self,
        body: str = '{"outputs": "ok"}',
        status: int = 200,
        headers: dict[str, str] | None = None,
    ) -> None:
        self.body = body
        self.status = status
        self.headers = headers
        self.form: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.form.update(parse_qsl(request.content.decode(), keep_blank_values=True))
        return httpx.Response(self.status, content=self.body.encode(), headers=self.headers)


def replace_environment(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)


def api_base_value(api_base: str | None) -> JsonValue:
    try:
        value = VLLMModelInfo.get_api_base(api_base)
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "value": value}


def models_value(api_base: str | None) -> JsonValue:
    try:
        VLLMModelInfo().get_models(None, api_base)
    except ValueError:
        return {"ok": False, "status": None}
    raise ValueError("the vllm models lookup resolved a key")


def model_info_value(call: InfoCall) -> JsonValue:
    replace_environment(call["env"])
    headers: dict[str, JsonValue] = INFO_VIEW.validate_environment(
        call["headers"], call["model"], [], {}, {}, call.get("api_key")
    )
    error = INFO_VIEW.get_error_class("boom", 418, {})
    return {
        "model": call["model"],
        "api_base": api_base_value(call.get("api_base")),
        "models": models_value(call.get("api_base")),
        "headers": headers,
        "error": {"status": error.status_code},
    }


def passthrough_value(call: PassCall) -> JsonValue:
    replace_environment(call["env"])
    streaming = PASS_VIEW.is_streaming_request(call["endpoint"], call["request_data"])
    try:
        url, base = PASS_VIEW.get_complete_url(
            call.get("api_base"), None, "m", call["endpoint"], call.get("query"), {}
        )
    except ValueError:
        return {"streaming": streaming, "result": {"ok": False, "status": None}}
    return {"streaming": streaming, "result": {"ok": True, "url": str(url), "base": base}}


def mapped_value(case: MapCase) -> JsonValue:
    options: dict[str, JsonValue] = {}
    optional = case.get("optional")
    if optional is not None:
        options.update(optional)
    PETALS_VIEW.map_openai_params(case["non_default"], options, "m", drop_params=False)
    return {"options": options}


def validated_value(case: ValidateCase) -> JsonValue:
    given = TypeAdapter(dict[str, JsonValue]).validate_python(case["headers"])
    key = case["api_key"]
    returned: dict[str, JsonValue] = PETALS_VIEW.validate_environment(
        case["headers"], "m", [], {}, {}, key
    )
    return {"headers": returned, "given": given, "api_key": key}


def params_value(cases: list[MapCase], validate: list[ValidateCase]) -> JsonValue:
    error = PETALS_VIEW.get_error_class("boom", 418, {})
    supported: list[JsonValue] = [*PETALS_VIEW.get_supported_openai_params("m")]
    return {
        "config": PETALS_VIEW.get_config(),
        "supported": supported,
        "mapped": [mapped_value(case) for case in cases],
        "validated": [validated_value(case) for case in validate],
        "error": {"status": error.status_code},
    }


# PetalsConfig.__init__ sets the class max_new_tokens that get_config reads (transformation.py L60)
def apply_max_new_tokens(values: dict[str, str] | None) -> None:
    if values is not None and "DEFAULT_MAX_TOKENS" in values:
        PetalsConfig(max_new_tokens=int(values["DEFAULT_MAX_TOKENS"]))


def request_value(call: RequestCase) -> JsonValue:
    responder = Responder()
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(responder.handle)))
    litellm.completion(
        model=f"petals/{call['model']}",
        messages=call["messages"],
        api_base=call["api_base"],
        client=client,
        max_tokens=call.get("max_tokens"),
        temperature=call.get("temperature"),
        top_p=call.get("top_p"),
        stream=call.get("stream"),
    )
    return {"ok": True, "form": responder.form}


def reply_value(call: ReplyCase) -> JsonValue:
    responder = Responder(call["body"], call["status"], call["headers"])
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(responder.handle)))
    try:
        # litellm.completion's decorated return is Any to mypy, so its type is named here.
        response: (
            ModelResponse | CustomStreamWrapper | Awaitable[ModelResponse | CustomStreamWrapper]
        ) = litellm.completion(
            model=f"petals/{call['model']}",
            messages=call["messages"],
            api_base="http://petals.test/v1/generate",
            client=client,
        )
    except openai.APIStatusError as error:
        return {"ok": False, "status": error.status_code}
    except openai.APIConnectionError:
        return {"ok": False, "status": 500}
    reply = TypeAdapter(ModelResponse).validate_python(response)
    usage = TypeAdapter(ReplyDump).validate_json(reply.model_dump_json())["usage"]
    return {
        "ok": True,
        "model": reply.model,
        "created": reply.created,
        "choices": [
            {"index": c.index, "content": c.message.content, "finish_reason": c.finish_reason}
            for c in reply.choices
        ],
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    op = input_value["op"]
    if op == "vllm_model_info":
        calls = TypeAdapter(list[InfoCall]).validate_python(input_value["calls"])
        return {"calls": [model_info_value(call) for call in calls]}
    if op == "vllm_passthrough":
        calls = TypeAdapter(list[PassCall]).validate_python(input_value["calls"])
        return {"calls": [passthrough_value(call) for call in calls]}
    if op == "petals_params":
        env = TypeAdapter(dict[str, str] | None).validate_python(input_value.get("env"))
        apply_max_new_tokens(env)
        cases = TypeAdapter(list[MapCase]).validate_python(input_value["cases"])
        validate = TypeAdapter(list[ValidateCase]).validate_python(input_value["validate"])
        return params_value(cases, validate)
    if op == "petals_request":
        env = TypeAdapter(dict[str, str] | None).validate_python(input_value.get("env"))
        apply_max_new_tokens(env)
        requests = TypeAdapter(list[RequestCase]).validate_python(input_value["requests"])
        custom = input_value.get("custom_prompt_dict")
        if custom is not None:
            litellm.custom_prompt_dict = TypeAdapter(
                dict[str, dict[str, JsonValue]]
            ).validate_python(custom)
        return {"requests": [request_value(call) for call in requests]}
    if op == "petals_reply":
        replies = TypeAdapter(list[ReplyCase]).validate_python(input_value["replies"])
        return {"replies": [reply_value(call) for call in replies]}
    raise ValueError("unknown operation")
