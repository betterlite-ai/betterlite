import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

MANAGED_ENV = ("MANUS_API_BASE", "MANUS_API_KEY")
RESPONSES_URL = "https://api.manus.im/v1/responses"


class Sdk(TypedDict):
    api_base: str | None
    api_key: str | None


class UrlProbe(TypedDict):
    api_base: str | None


Body: TypeAlias = dict[str, JsonValue]
Create: TypeAlias = Callable[[str, str, Body, GenericLiteLLMParams, Body], Body]
Reply: TypeAlias = Callable[[str, httpx.Response, Logging], ResponsesAPIResponse]
Retrieve: TypeAlias = Callable[[httpx.Response, Logging], ResponsesAPIResponse]
Item: TypeAlias = Callable[[str, str, GenericLiteLLMParams, Body], tuple[str, Body]]
Url: TypeAlias = Callable[[str | None, Body], str]


class CreateProbe(TypedDict):
    model: str
    input: str
    optional_params: Body


class ReplyProbe(TypedDict):
    body: str
    status: int
    headers: dict[str, str]


class GetProbe(TypedDict):
    response_id: str
    api_base: str


class UrlsInput(TypedDict):
    op: Literal["urls"]
    environment: dict[str, str]
    sdk: Sdk
    probes: list[UrlProbe]


class CreateInput(TypedDict):
    op: Literal["create_requests"]
    environment: dict[str, str]
    sdk: Sdk
    probes: list[CreateProbe]


class RepliesInput(TypedDict):
    op: Literal["replies"]
    environment: dict[str, str]
    sdk: Sdk
    now: int
    probes: list[ReplyProbe]


class GetInput(TypedDict):
    op: Literal["get_requests"]
    probes: list[GetProbe]


class UsageFields(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int


class ReplyFields(TypedDict):
    id: str
    created_at: int
    reasoning: dict[str, JsonValue] | None
    text: dict[str, JsonValue] | None
    output: list[JsonValue]
    usage: UsageFields | None


class HiddenParams(TypedDict):
    headers: dict[str, JsonValue]
    additional_headers: dict[str, JsonValue]


class PostCall(TypedDict):
    original_response: JsonValue
    additional_args: dict[str, JsonValue]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None


class Hidden(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    params: HiddenParams = Field(alias="_hidden_params")


class Recorded(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: PostCall


Input = UrlsInput | CreateInput | RepliesInput | GetInput


def activate(environment: dict[str, str]) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def activate_sdk(sdk: Sdk) -> None:
    litellm.api_key = sdk["api_key"]
    litellm.api_base = sdk["api_base"]


# LiteLLM types the request conversions as bare dicts, so the callables fix the shape
class ManusConfig(Protocol):
    @property
    def transform_responses_api_request(self) -> Create: ...  # ast-grep-ignore

    @property
    def transform_response_api_response(self) -> Reply: ...  # ast-grep-ignore

    @property
    def transform_get_response_api_response(self) -> Retrieve: ...  # ast-grep-ignore

    @property
    def transform_get_response_api_request(self) -> Item: ...  # ast-grep-ignore

    @property
    def get_complete_url(self) -> Url: ...


def config() -> ManusConfig:
    found = ProviderConfigManager.get_provider_responses_api_config(provider=LlmProviders.MANUS)
    if found is None:
        raise ValueError("no Manus responses config")
    return found


def create_body(found: ManusConfig, probe: CreateProbe) -> Body:
    return found.transform_responses_api_request(
        probe["model"],
        probe["input"],
        probe["optional_params"],
        GenericLiteLLMParams(),
        {},
    )


def url_value(found: ManusConfig, probe: UrlProbe) -> JsonValue:
    try:
        return {"url": found.get_complete_url(probe["api_base"], {})}
    except ValueError:
        return {"raised": True}


def urls(input_value: UrlsInput) -> JsonValue:
    activate(input_value["environment"])
    activate_sdk(input_value["sdk"])
    found = config()
    return {"urls": [url_value(found, probe) for probe in input_value["probes"]]}


def create_value(found: ManusConfig, probe: CreateProbe) -> JsonValue:
    try:
        return {"body": create_body(found, probe)}
    except (ValueError, TypeError, KeyError):
        return {"raised": True}


def creates(input_value: CreateInput) -> JsonValue:
    activate(input_value["environment"])
    activate_sdk(input_value["sdk"])
    found = config()
    return {"requests": [create_value(found, probe) for probe in input_value["probes"]]}


def logging_object() -> Logging:
    started = datetime(2026, 1, 1, tzinfo=UTC)
    return Logging(
        model="manus-1.6",
        messages=None,
        stream=False,
        call_type="responses",
        start_time=started,
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def raw_response(probe: ReplyProbe) -> httpx.Response:
    request = httpx.Request("POST", RESPONSES_URL)
    stream = httpx.ByteStream(probe["body"].encode())
    response = httpx.Response(
        probe["status"], stream=stream, headers=probe["headers"], request=request
    )
    response.read()
    return response


def fields_view(reply: BaseModel, logging_obj: Logging) -> JsonValue:
    record = TypeAdapter(ReplyFields).validate_json(reply.model_dump_json())
    hidden = Hidden.model_validate(reply)
    post = Recorded.model_validate(logging_obj).model_call_details
    usage = record["usage"]
    usage_value: JsonValue = None
    if usage is not None:
        usage_value = {
            "input_tokens": usage["input_tokens"],
            "output_tokens": usage["output_tokens"],
            "total_tokens": usage["total_tokens"],
        }
    return {
        "id": record["id"],
        "created_at": record["created_at"],
        "reasoning": record["reasoning"],
        "text": record["text"],
        "output": record["output"],
        "usage": usage_value,
        "headers": hidden.params["headers"],
        "additional_headers": hidden.params["additional_headers"],
        "post_call": {
            "original_response": post["original_response"],
            "additional_args": post["additional_args"],
        },
    }


def reply_value(
    found: ManusConfig, probe: ReplyProbe, endpoint: Literal["create", "retrieve"]
) -> JsonValue:
    logging_obj = logging_object()
    try:
        if endpoint == "create":
            response = raw_response(probe)
            reply = found.transform_response_api_response("manus-1.6", response, logging_obj)
        else:
            reply = found.transform_get_response_api_response(raw_response(probe), logging_obj)
    except (ValueError, TypeError, KeyError):
        return {"raised": True}
    except Exception as error:
        status = Failure.model_validate(error).status_code
        if status is None:
            raise
        return {"error": {"status": status}}
    return fields_view(reply, logging_obj)


def replies(input_value: RepliesInput) -> JsonValue:
    activate(input_value["environment"])
    activate_sdk(input_value["sdk"])
    found = config()
    values: list[JsonValue] = [
        {
            "create": reply_value(found, probe, "create"),
            "retrieve": reply_value(found, probe, "retrieve"),
        }
        for probe in input_value["probes"]
    ]
    return {"replies": values}


def get_value(found: ManusConfig, probe: GetProbe) -> JsonValue:
    try:
        url, data = found.transform_get_response_api_request(
            probe["response_id"], probe["api_base"], GenericLiteLLMParams(), {}
        )
    except (ValueError, TypeError, KeyError):
        return {"raised": True}
    return {"url": url, "data": data}


def gets(input_value: GetInput) -> JsonValue:
    found = config()
    return {"requests": [get_value(found, probe) for probe in input_value["probes"]]}


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "urls":
        return urls(input_value)
    if input_value["op"] == "create_requests":
        return creates(input_value)
    if input_value["op"] == "replies":
        return replies(input_value)
    return gets(input_value)
