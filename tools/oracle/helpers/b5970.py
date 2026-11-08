import json
import os
from collections.abc import Generator
from contextlib import ExitStack, contextmanager
from typing import Protocol, TypeAlias, TypedDict, cast
from unittest.mock import patch
from urllib.parse import urlencode

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.llms.watsonx import common_utils
from litellm.llms.watsonx.chat.transformation import IBMWatsonXChatConfig
from litellm.llms.watsonx.common_utils import (
    WatsonXAIError,
    _generate_watsonx_token,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _get_api_params,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)
from litellm.types.llms.watsonx import WatsonXAPIParams

Json: TypeAlias = (
    bool | int | float | str | list["Json"] | dict[str, "Json"] | None
)
Object: TypeAlias = dict[str, Json]
FIELDS = {"env", "routes", "steps"}
VARIABLES = (
    "WATSONX_IAM_URL",
    "WX_API_KEY",
    "WATSONX_API_KEY",
    "WATSONX_APIKEY",
    "WATSONX_ZENAPIKEY",
    "WATSONX_TOKEN",
    "WATSONX_API_BASE",
    "WATSONX_URL",
    "WX_URL",
    "WML_URL",
    "WATSONX_PROJECT_ID",
    "WX_PROJECT_ID",
    "PROJECT_ID",
    "WATSONX_REGION",
    "WX_REGION",
    "REGION",
    "WATSONX_DEPLOYMENT_SPACE_ID",
    "WATSONX_SPACE_ID",
    "WX_SPACE_ID",
    "SPACE_ID",
)
DEFAULT_FAILURE_STATUS = 500


class ApiParamsResolver(Protocol):
    def __call__(
        self, params: Object, model: str | None = None
    ) -> WatsonXAPIParams: ...


class TokenGenerator(Protocol):
    def __call__(self, api_key: str | None, token: str | None) -> str: ...


class CredentialResolver(Protocol):
    def __call__(
        self, optional_params: Object, api_key: str | None, api_base: str | None
    ) -> Object: ...


class WatsonxConfig(Protocol):
    def _get_base_url(self, api_base: str | None) -> str: ...

    def _add_api_version_to_url(self, url: str, api_version: str | None) -> str: ...

    def _prepare_payload(self, model: str, api_params: WatsonXAPIParams) -> Object: ...

    def validate_environment(  # noqa: PLR0913, PLR0917
        self,
        headers: dict[str, str],
        model: str,
        messages: list[Json],
        optional_params: Object,
        litellm_params: Object,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> Object: ...


GET_API_PARAMS = cast("ApiParamsResolver", _get_api_params)
GENERATE_WATSONX_TOKEN = cast("TokenGenerator", _generate_watsonx_token)
GET_WATSONX_CREDENTIALS = cast(
    "CredentialResolver", common_utils.IBMWatsonXMixin.get_watsonx_credentials
)
CONFIG = cast("WatsonxConfig", IBMWatsonXChatConfig())


class Clock:
    def __init__(self) -> None:
        self.now = 0.0

    def read(self) -> float:
        return self.now


class Call(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    data: dict[str, str]


class Transport:
    def __init__(self, routes: list[Json]) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = expect_object(entry)
            self.replies[expect_text(fields["route"])] = [
                expect_object(reply) for reply in expect_list(fields["replies"])
            ]
        self.requests: list[Json] = []

    def answer(self, call: Call) -> httpx.Response:
        route = f"{call['method']} {call['url']}"
        sent: Object = {
            "method": call["method"],
            "url": call["url"],
            "headers": {name.lower(): value for name, value in call["headers"].items()},
            "body": urlencode(call["data"]),
        }
        self.requests.append(sent)
        replies = self.replies.get(route, [])
        reply = (
            replies.pop(0)
            if replies
            else {"status": DEFAULT_FAILURE_STATUS, "body": ""}
        )
        if reply.get("failure") is True:
            raise httpx.ConnectError("connection refused")
        return httpx.Response(
            expect_integer(reply["status"]),
            text=expect_text(reply["body"]),
            request=httpx.Request(call["method"], call["url"]),
        )

    def take(self) -> list[Json]:
        taken = self.requests
        self.requests = []
        return taken


class Client:
    def __init__(self, transport: Transport) -> None:
        self.transport = transport

    def post(
        self,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        data: dict[str, str] | None = None,
        **_options: object,
    ) -> httpx.Response:
        call: Call = {
            "method": "POST",
            "url": url,
            "headers": dict(headers or {}),
            "data": dict(data or {}),
        }
        return self.transport.answer(call)


class Input(TypedDict):
    env: Object
    routes: list[Json]
    steps: list[Json]


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_integer(value: Json) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError("expected an integer")
    return value


def expect_number(value: Json) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError("expected a number")
    return float(value)


def expect_text_or_none(value: Json) -> str | None:
    if value is None:
        return None
    return expect_text(value)


def error_tag(error: Exception) -> Json:
    if isinstance(error, WatsonXAIError):
        return {"error": "watsonx", "status": error.status_code}
    if isinstance(error, httpx.HTTPStatusError):
        return {"error": "iam_status", "status": error.response.status_code}
    if isinstance(error, httpx.TransportError):
        return {"error": "iam_transport"}
    if isinstance(error, (json.JSONDecodeError, KeyError, TypeError)):
        return {"error": "iam_reply"}
    if isinstance(error, ValueError) and str(error) == "API key is required":
        return {"error": "api_key_required"}
    raise error


def api_params_result(params: Object, model: Json) -> Json:
    try:
        found = GET_API_PARAMS(params=params, model=expect_text_or_none(model))
    except WatsonXAIError as error:
        return error_tag(error)
    result: Object = {
        "api_params": cast("Json", dict(found)),
        "params": params,
    }
    return result


def credentials_result(params: Object, api_key: Json, api_base: Json) -> Json:
    try:
        found = GET_WATSONX_CREDENTIALS(
            optional_params=params,
            api_key=expect_text_or_none(api_key),
            api_base=expect_text_or_none(api_base),
        )
    except WatsonXAIError as error:
        return error_tag(error)
    result: Object = {
        "api_key": found["api_key"],
        "api_base": found["api_base"],
        "token": found["token"],
        "params": params,
    }
    return result


def base_url_result(api_base: Json) -> Json:
    try:
        return {
            "url": CONFIG._get_base_url(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                api_base=expect_text_or_none(api_base)
            )
        }
    except WatsonXAIError as error:
        return error_tag(error)


def api_version_result(url: str, api_version: Json) -> Json:
    version = CONFIG._add_api_version_to_url(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        url=url, api_version=expect_text_or_none(api_version)
    )
    return {"url": version}


def payload_result(model: str, api_params: Object) -> Json:
    return CONFIG._prepare_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        model=model, api_params=cast("WatsonXAPIParams", api_params)
    )


def authorization_result(headers: Object, params: Object, api_key: Json) -> Json:
    try:
        resolved = CONFIG.validate_environment(
            headers={name: expect_text(value) for name, value in headers.items()},
            model="watsonx-model",
            messages=[],
            optional_params=params,
            litellm_params={},
            api_key=expect_text_or_none(api_key),
        )
    except WatsonXAIError as error:
        return error_tag(error)
    except (
        httpx.HTTPStatusError,
        httpx.TransportError,
        ValueError,
        KeyError,
        TypeError,
        json.JSONDecodeError,
    ) as error:
        return error_tag(error)
    result: Object = {
        "headers": {name.lower(): value for name, value in resolved.items()},
        "params": params,
    }
    return result


def generate_result(api_key: Json, token: Json) -> Json:
    try:
        found = GENERATE_WATSONX_TOKEN(
            api_key=expect_text_or_none(api_key),
            token=expect_text_or_none(token),
        )
    except WatsonXAIError as error:
        return error_tag(error)
    except (
        httpx.HTTPStatusError,
        httpx.TransportError,
        ValueError,
        KeyError,
        TypeError,
        json.JSONDecodeError,
    ) as error:
        return error_tag(error)
    return {"token": found}


def run_step(step: Object, clock: Clock) -> Json:
    operation = expect_text(step["op"])
    result: Json = None
    match operation:
        case "api_params":
            result = api_params_result(expect_object(step["params"]), step.get("model"))
        case "credentials":
            result = credentials_result(
                expect_object(step["params"]), step.get("api_key"), step.get("api_base")
            )
        case "base_url":
            result = base_url_result(step.get("api_base"))
        case "api_version":
            result = api_version_result(expect_text(step["url"]), step.get("api_version"))
        case "payload":
            result = payload_result(
                expect_text(step["model"]), expect_object(step["api_params"])
            )
        case "iam_url":
            result = {"url": common_utils.get_watsonx_iam_url()}
        case "authorization":
            result = authorization_result(
                expect_object(step["headers"]),
                expect_object(step["params"]),
                step.get("api_key"),
            )
        case "generate":
            result = generate_result(step.get("api_key"), step.get("token"))
        case "advance":
            clock.now += expect_number(step["seconds"])
        case _:
            raise ValueError(f"unknown b5970 step: {operation}")
    return result


def execute(input_value: Object, clock: Clock, transport: Transport) -> Json:
    steps: list[Json] = []
    for raw_step in expect_list(input_value["steps"]):
        step = expect_object(raw_step)
        result = run_step(step, clock)
        steps.append({"result": result, "requests": transport.take()})
    return {"steps": steps}


@contextmanager
def patched_world(env: Object, transport: Transport, clock: Clock) -> Generator[None]:
    saved_environment = {
        name: value for name, value in os.environ.items() if name in VARIABLES
    }
    saved_client = litellm.module_level_client
    saved_secret_client = litellm.secret_manager_client
    saved_system = litellm._key_management_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    for name in VARIABLES:
        os.environ.pop(name, None)
    for name, value in env.items():
        os.environ[name] = expect_text(value)
    cache = InMemoryCache(clock=clock.read)
    try:
        with ExitStack() as stack:
            stack.enter_context(patch.object(common_utils, "iam_token_cache", cache))
            stack.enter_context(
                patch.object(litellm, "module_level_client", Client(transport))
            )
            yield
    finally:
        for name in VARIABLES:
            os.environ.pop(name, None)
        os.environ.update(saved_environment)
        litellm.module_level_client = saved_client
        litellm.secret_manager_client = saved_secret_client
        litellm._key_management_system = saved_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    clock = Clock()
    transport = Transport(expect_list(input_value["routes"]))
    with patched_world(expect_object(input_value["env"]), transport, clock):
        return execute(input_value, clock, transport)
