import os
from datetime import UTC, datetime
from decimal import Decimal
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.exceptions import AuthenticationError
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.xai import oauth as xai_oauth
from litellm.llms.xai.oauth import XAIOAuthAuthenticator, XAIOAuthError
from litellm.llms.xai.responses.transformation import XAIResponsesAPIConfig
from litellm.types.llms.openai import (
    ResponsesAPIOptionalRequestParams as RequestParams,
)
from litellm.types.llms.openai import ResponsesAPIStreamingResponse
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

CONFIG: Final[XAIResponsesAPIConfig] = XAIResponsesAPIConfig.__new__(
    XAIResponsesAPIConfig
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class ResponsesConfig(Protocol):
    def map_openai_params(
        self,
        response_api_optional_params: RequestParams,
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        litellm_params: GenericLiteLLMParams | None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_streaming_response(
        self, model: str, parsed_chunk: dict[str, JsonValue], logging_obj: Logging
    ) -> ResponsesAPIStreamingResponse: ...


class UsageView(TypedDict, total=False):
    cost: float | None
    cost_in_usd_ticks: JsonValue


class ResponseView(TypedDict, total=False):
    usage: UsageView


class EventView(TypedDict, total=False):
    response: ResponseView


USAGE: Final[TypeAdapter[UsageView]] = TypeAdapter(UsageView)
EVENT: Final[TypeAdapter[EventView]] = TypeAdapter(EventView)


class Tools(TypedDict):
    step: Literal["tools"]
    params: RequestParams


class Url(TypedDict):
    step: Literal["url"]
    api_base: str | None
    litellm_api_base: str | None
    api_key: str | None
    use_xai_oauth: bool


class Auth(TypedDict):
    step: Literal["auth"]
    api_key: str | None
    use_xai_oauth: bool
    headers: dict[str, str]
    oauth: str | None


class Reply(TypedDict):
    step: Literal["reply"]
    status: int
    body: str
    now: int


class Event(TypedDict):
    step: Literal["event"]
    event: dict[str, JsonValue]


class Capability(TypedDict):
    step: Literal["capability"]


Step: TypeAlias = Tools | Url | Auth | Reply | Event | Capability


class Input(TypedDict):
    env: dict[str, str]
    xai_key: str | None
    generic_key: str | None
    steps: list[Step]


class OauthFixture(XAIOAuthAuthenticator):
    token: str = ""

    # the fixture never reads the token store the real method would
    def get_access_token(self) -> str:
        if not OauthFixture.token:
            raise XAIOAuthError("fixture failure")
        return OauthFixture.token


def logging_object() -> Logging:
    return Logging(
        model="grok-4",
        messages=None,
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def money(value: float | None) -> str | None:
    if value is None:
        return None
    return format(Decimal(repr(value)).normalize(), "f")


def usage_value(usage: UsageView | None) -> dict[str, JsonValue]:
    if usage is None:
        return {"cost": None, "ticks": None}
    return {"cost": money(usage.get("cost")), "ticks": usage.get("cost_in_usd_ticks")}


def tools_value(config: ResponsesConfig, step: Tools) -> JsonValue:
    params = config.map_openai_params(
        step["params"], "grok-4", drop_params=False
    )
    return JSON.validate_python(params)


def url_value(config: ResponsesConfig, step: Url) -> JsonValue:
    litellm.api_base = step["litellm_api_base"]
    litellm_params: dict[str, JsonValue] = {
        "api_key": step["api_key"],
        "use_xai_oauth": step["use_xai_oauth"],
    }
    return config.get_complete_url(step["api_base"], litellm_params)


def auth_value(config: ResponsesConfig, step: Auth) -> JsonValue:
    params = GenericLiteLLMParams(
        api_key=step["api_key"], use_xai_oauth=step["use_xai_oauth"]
    )
    given = dict(step["headers"])
    token = step["oauth"]
    OauthFixture.token = "" if token is None else token
    original = xai_oauth.XAIOAuthAuthenticator
    xai_oauth.XAIOAuthAuthenticator = OauthFixture
    try:
        headers = config.validate_environment(given, "grok-4", params)
    except ValueError:
        return {"raised": None}
    except AuthenticationError as error:
        return {"raised": error.status_code}
    finally:
        xai_oauth.XAIOAuthAuthenticator = original
    return JSON.validate_python(headers)


def reply_value(step: Reply) -> JsonValue:
    request = httpx.Request("POST", "https://api.x.ai/v1/responses")
    response = httpx.Response(
        step["status"], content=step["body"].encode(), request=request
    )
    try:
        reply = CONFIG.transform_response_api_response(
            "grok-4", response, logging_object()
        )
    except OpenAIError as error:
        return {"raised": error.status_code}
    if reply.usage is None:
        return usage_value(None)
    return usage_value(USAGE.validate_json(reply.usage.model_dump_json()))


def event_value(config: ResponsesConfig, step: Event) -> JsonValue:
    event = config.transform_streaming_response(
        "grok-4", step["event"], logging_object()
    )
    view = EVENT.validate_json(event.model_dump_json())
    response = view.get("response")
    usage = None if response is None else response.get("usage")
    value = usage_value(usage)
    value["class"] = type(event).__name__
    return value


def step_value(step: Step) -> JsonValue:
    match step["step"]:
        case "tools":
            return tools_value(CONFIG, step)
        case "url":
            return url_value(CONFIG, step)
        case "auth":
            return auth_value(CONFIG, step)
        case "reply":
            return reply_value(step)
        case "event":
            return event_value(CONFIG, step)
        case "capability":
            return {"native_websocket": CONFIG.supports_native_websocket()}


def run(input_value: Input) -> JsonValue:
    for name in ("XAI_API_KEY", "XAI_API_BASE", "XAI_OAUTH_API_BASE"):
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    litellm.xai_key = input_value["xai_key"]
    litellm.api_key = input_value["generic_key"]
    litellm.api_base = None
    return [step_value(step) for step in input_value["steps"]]
