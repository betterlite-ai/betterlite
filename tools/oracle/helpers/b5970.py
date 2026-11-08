import json
import os
from collections.abc import Callable, Generator
from contextlib import ExitStack, contextmanager
from typing import TypeAlias, cast
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
from litellm.types.llms.watsonx import WatsonXAPIParams  # noqa: TC002

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Object: TypeAlias = dict[str, Json]
FIELDS = {"env", "routes", "steps"}
VARIABLES = """
WATSONX_IAM_URL WX_API_KEY WATSONX_API_KEY WATSONX_APIKEY WATSONX_ZENAPIKEY
WATSONX_TOKEN WATSONX_API_BASE WATSONX_URL WX_URL WML_URL
WATSONX_PROJECT_ID WX_PROJECT_ID PROJECT_ID WATSONX_REGION WX_REGION REGION
WATSONX_DEPLOYMENT_SPACE_ID WATSONX_SPACE_ID WX_SPACE_ID SPACE_ID
""".split()  # noqa: SIM905

HELPER_ERRORS = (
    WatsonXAIError,
    httpx.HTTPStatusError,
    httpx.TransportError,
    ValueError,
    KeyError,
    TypeError,
    json.JSONDecodeError,
)
CONFIG = IBMWatsonXChatConfig()
GET_BASE_URL = cast("Callable[..., str]", CONFIG._get_base_url)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
ADD_API_VERSION = cast("Callable[..., str]", CONFIG._add_api_version_to_url)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
PREPARE_PAYLOAD = cast("Callable[..., Object]", CONFIG._prepare_payload)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
VALIDATE_ENVIRONMENT = cast("Callable[..., Object]", CONFIG.validate_environment)


class Transport:
    def __init__(self, routes: list[Json]) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = expect_object(entry)
            self.replies[expect_text(fields["route"])] = [
                expect_object(reply) for reply in expect_list(fields["replies"])
            ]
        self.requests: list[Json] = []

    def post(
        self,
        url: str,
        *,
        headers: dict[str, str] | None = None,
        data: dict[str, str] | None = None,
        **_options: object,
    ) -> httpx.Response:
        route = f"POST {url}"
        sent: Object = {
            "method": "POST",
            "url": url,
            "headers": {name.lower(): value for name, value in (headers or {}).items()},
            "body": urlencode(data or {}),
        }
        self.requests.append(sent)
        replies = self.replies.get(route, [])
        reply = replies.pop(0) if replies else {"status": 500, "body": ""}
        if reply.get("failure") is True:
            raise httpx.ConnectError("connection refused")
        status = reply["status"]
        if not isinstance(status, int) or isinstance(status, bool):
            raise TypeError("expected an integer")
        return httpx.Response(
            status,
            text=expect_text(reply["body"]),
            request=httpx.Request("POST", url),
        )


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


def expect_text_or_none(value: Json) -> str | None:
    return None if value is None else expect_text(value)


def expect_number(value: Json) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError("expected a number")
    return float(value)


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
        found = cast("Callable[..., WatsonXAPIParams]", _get_api_params)(
            params=params, model=expect_text_or_none(model)
        )
    except WatsonXAIError as error:
        return error_tag(error)
    return {"api_params": cast("Json", dict(found)), "params": params}


def credentials_result(params: Object, api_key: Json, api_base: Json) -> Json:
    try:
        found = cast(
            "Callable[..., Object]", common_utils.IBMWatsonXMixin.get_watsonx_credentials
        )(
            optional_params=params,
            api_key=expect_text_or_none(api_key),
            api_base=expect_text_or_none(api_base),
        )
    except WatsonXAIError as error:
        return error_tag(error)
    return {
        "api_key": found["api_key"],
        "api_base": found["api_base"],
        "token": found["token"],
        "params": params,
    }


def authorization_result(headers: Object, params: Object, api_key: Json) -> Json:
    try:
        resolved = VALIDATE_ENVIRONMENT(
            headers={name: expect_text(value) for name, value in headers.items()},
            model="watsonx-model",
            messages=[],
            optional_params=params,
            litellm_params={},
            api_key=expect_text_or_none(api_key),
        )
    except HELPER_ERRORS as error:
        return error_tag(error)
    return {
        "headers": {name.lower(): value for name, value in resolved.items()},
        "params": params,
    }


def generate_result(api_key: Json, token: Json) -> Json:
    try:
        found = cast("Callable[..., str]", _generate_watsonx_token)(
            api_key=expect_text_or_none(api_key),
            token=expect_text_or_none(token),
        )
    except HELPER_ERRORS as error:
        return error_tag(error)
    return {"token": found}


def run_step(step: Object, now: list[float]) -> Json:  # noqa: C901
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
            try:
                result = {
                    "url": GET_BASE_URL(api_base=expect_text_or_none(step.get("api_base")))
                }
            except WatsonXAIError as error:
                result = error_tag(error)
        case "api_version":
            result = {
                "url": ADD_API_VERSION(
                    url=expect_text(step["url"]),
                    api_version=expect_text_or_none(step.get("api_version")),
                )
            }
        case "payload":
            result = PREPARE_PAYLOAD(
                model=expect_text(step["model"]),
                api_params=cast("WatsonXAPIParams", expect_object(step["api_params"])),
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
            now[0] = now[0] + expect_number(step["seconds"])
        case _:
            raise ValueError(f"unknown b5970 step: {operation}")
    return result


@contextmanager
def patched_world(
    env: Object, transport: Transport, now: list[float]
) -> Generator[None]:
    saved_environment = {
        name: value for name, value in os.environ.items() if name in VARIABLES
    }
    saved_client = litellm.module_level_client
    for name in VARIABLES:
        os.environ.pop(name, None)
    for name, value in env.items():
        os.environ[name] = expect_text(value)
    cache = InMemoryCache(clock=lambda: now[0])
    try:
        with ExitStack() as stack:
            stack.enter_context(patch.object(common_utils, "iam_token_cache", cache))
            stack.enter_context(patch.object(litellm, "module_level_client", transport))
            yield
    finally:
        for name in VARIABLES:
            os.environ.pop(name, None)
        os.environ.update(saved_environment)
        litellm.module_level_client = saved_client


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    now = [0.0]
    transport = Transport(expect_list(input_value["routes"]))
    with patched_world(expect_object(input_value["env"]), transport, now):
        steps: list[Json] = []
        for raw_step in expect_list(input_value["steps"]):
            step = expect_object(raw_step)
            result = run_step(step, now)
            requests = transport.requests
            transport.requests = []
            steps.append({"result": result, "requests": requests})
        return {"steps": steps}
