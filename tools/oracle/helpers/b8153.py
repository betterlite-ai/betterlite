import os
from typing import Literal, NotRequired, Protocol, TypedDict
from urllib.parse import parse_qsl

import httpx
import litellm
import openai
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.petals.completion.transformation import PetalsConfig
from litellm.llms.vllm.common_utils import VLLMModelInfo
from litellm.llms.vllm.passthrough.transformation import VLLMPassthroughConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

PETALS_BASE = "http://petals.test/v1/generate"


class NoMessages(list[AllMessageValues]):
    pass


class ErrorView(Protocol):
    status_code: int

    def __str__(self) -> str: ...


class Info(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, JsonValue]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, JsonValue]
    ) -> ErrorView: ...


class Passthrough(Protocol):
    def is_streaming_request(self, endpoint: str, request_data: dict[str, JsonValue]) -> bool: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        endpoint: str,
        request_query_params: dict[str, JsonValue] | None,
        litellm_params: dict[str, JsonValue],
    ) -> tuple[httpx.URL, str]: ...


class Petals(Protocol):
    def get_config(self) -> dict[str, JsonValue]: ...

    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, JsonValue]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, JsonValue]
    ) -> ErrorView: ...


INFO: Info = VLLMModelInfo()
PASS: Passthrough = VLLMPassthroughConfig()
PETALS: Petals = PetalsConfig()


class InfoCall(TypedDict, total=False):
    model: str
    api_base: str | None
    env: dict[str, str]
    api_key: str | None
    headers: dict[str, str]


class InfoInput(TypedDict):
    op: Literal["vllm_model_info"]
    calls: list[InfoCall]


class PassCall(TypedDict, total=False):
    endpoint: str
    api_base: str | None
    env: dict[str, str]
    query: dict[str, JsonValue] | None
    request_data: dict[str, JsonValue]


class PassInput(TypedDict):
    op: Literal["vllm_passthrough"]
    calls: list[PassCall]


class MapCase(TypedDict, total=False):
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]


class ValidateCase(TypedDict, total=False):
    headers: dict[str, str]
    api_key: str | None


class ParamsInput(TypedDict):
    op: Literal["petals_params"]
    cases: list[MapCase]
    validate: list[ValidateCase]


class RequestCase(TypedDict, total=False):
    model: str
    messages: list[dict[str, JsonValue]]
    api_base: str | None
    max_tokens: int | None
    temperature: float | None
    top_p: float | None
    stream: bool | None


class RequestInput(TypedDict):
    op: Literal["petals_request"]
    custom_prompt_dict: NotRequired[dict[str, dict[str, JsonValue]]]
    requests: list[RequestCase]


class ReplyCase(TypedDict, total=False):
    model: str
    body: str
    status: int
    headers: dict[str, str]
    messages: list[dict[str, JsonValue]]


class ReplyInput(TypedDict):
    op: Literal["petals_reply"]
    replies: list[ReplyCase]


Input = InfoInput | PassInput | ParamsInput | RequestInput | ReplyInput
INPUT = TypeAdapter(Input)


class ReplyMessage(TypedDict):
    content: str


class ReplyChoice(TypedDict):
    index: int
    finish_reason: str | None
    message: ReplyMessage


class Reply(TypedDict):
    model: str
    choices: list[ReplyChoice]


REPLY = TypeAdapter(Reply)


class Recorder:
    def __init__(self) -> None:
        self.form: dict[str, str] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.form = {}
        for name, value in parse_qsl(request.content.decode(), keep_blank_values=True):
            self.form[name] = value
        return httpx.Response(
            200, content=b'{"outputs": "ok"}', headers={"content-type": "application/json"}
        )


class Responder:
    def __init__(self, body: str, status: int, headers: dict[str, str] | None) -> None:
        self.body = body
        self.status = status
        self.headers = headers

    def handle(self, _request: httpx.Request) -> httpx.Response:
        return httpx.Response(self.status, content=self.body.encode(), headers=self.headers)


def replace_environment(values: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(values)


def api_base_value(api_base: str | None) -> JsonValue:
    try:
        return {"ok": True, "value": VLLMModelInfo.get_api_base(api_base)}
    except ValueError as error:
        return {"ok": False, "message": str(error)}


def models_value(api_base: str | None) -> JsonValue:
    try:
        return {"ok": True, "value": VLLMModelInfo().get_models(None, api_base)}
    except ValueError as error:
        return {"ok": False, "message": str(error)}


def model_info_value(call: InfoCall) -> JsonValue:
    replace_environment(call["env"])
    key = call.get("api_key")
    headers: dict[str, JsonValue] = INFO.validate_environment(
        call["headers"], call["model"], NoMessages(), {}, {}, api_key=key
    )
    error = INFO.get_error_class("boom", 418, {})
    return {
        "model": call["model"],
        "api_base": api_base_value(call.get("api_base")),
        "models": models_value(call.get("api_base")),
        "headers": headers,
        "error": {"status": error.status_code, "message": str(error)},
    }


def passthrough_value(call: PassCall) -> JsonValue:
    replace_environment(call["env"])
    streaming = PASS.is_streaming_request(call["endpoint"], call["request_data"])
    try:
        url, base = PASS.get_complete_url(
            call.get("api_base"), None, "m", call["endpoint"], call.get("query"), {}
        )
    except ValueError as error:
        return {"streaming": streaming, "result": {"ok": False, "message": str(error)}}
    return {"streaming": streaming, "result": {"ok": True, "url": str(url), "base": base}}


def mapped_value(case: MapCase) -> JsonValue:
    options: dict[str, JsonValue] = {}
    optional = case.get("optional")
    if optional is not None:
        options.update(optional)
    PETALS.map_openai_params(case["non_default"], options, "m", False)
    return {"options": options}


def validated_value(case: ValidateCase) -> JsonValue:
    headers: dict[str, str] = {}
    given = case.get("headers")
    if given is not None:
        headers.update(given)
    key = case.get("api_key")
    returned: dict[str, JsonValue] = PETALS.validate_environment(
        headers, "m", NoMessages(), {}, {}, api_key=key
    )
    return {"headers": returned, "given": headers, "api_key": key}


def params_value(call: ParamsInput) -> JsonValue:
    error = PETALS.get_error_class("boom", 418, {})
    return {
        "config": PETALS.get_config(),
        "supported": PETALS.get_supported_openai_params("m"),
        "mapped": [mapped_value(case) for case in call["cases"]],
        "validated": [validated_value(case) for case in call["validate"]],
        "error": {"status": error.status_code, "message": str(error)},
    }


def request_value(call: RequestCase) -> JsonValue:
    recorder = Recorder()
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle)))
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
    return {"ok": True, "form": recorder.form}


def reply_of(response: ModelResponse) -> JsonValue:
    parsed = REPLY.validate_json(response.model_dump_json())
    choices: list[JsonValue] = []
    for choice in parsed["choices"]:
        choices.append(
            {
                "index": choice["index"],
                "content": choice["message"]["content"],
                "finish_reason": choice["finish_reason"],
            }
        )
    return {
        "ok": True,
        "model": parsed["model"],
        "created": response.created,
        "choices": choices,
    }


def reply_value(call: ReplyCase) -> JsonValue:
    responder = Responder(call["body"], call["status"], call.get("headers"))
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(responder.handle)))
    try:
        raw = litellm.completion(
            model=f"petals/{call['model']}",
            messages=call["messages"],
            api_base=PETALS_BASE,
            client=client,
        )
    except openai.APIStatusError as error:
        return {"ok": False, "status": error.status_code, "message": str(error)}
    except openai.APIConnectionError as error:
        return {"ok": False, "status": 500, "message": str(error)}
    match raw:
        case ModelResponse():
            return reply_of(raw)
        case CustomStreamWrapper():
            raise ValueError("the petals reply came back streamed")


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed: Input = INPUT.validate_python(input_value)
    result: JsonValue
    if parsed["op"] == "vllm_model_info":
        result = {"calls": [model_info_value(call) for call in parsed["calls"]]}
    elif parsed["op"] == "vllm_passthrough":
        result = {"calls": [passthrough_value(call) for call in parsed["calls"]]}
    elif parsed["op"] == "petals_params":
        result = params_value(parsed)
    elif parsed["op"] == "petals_request":
        custom = parsed.get("custom_prompt_dict")
        if custom is not None:
            litellm.custom_prompt_dict = custom
        result = {"requests": [request_value(call) for call in parsed["requests"]]}
    else:
        result = {"replies": [reply_value(call) for call in parsed["replies"]]}
    return result
