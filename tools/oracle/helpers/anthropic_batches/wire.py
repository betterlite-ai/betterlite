import os
from datetime import UTC, datetime
from types import ModuleType
from typing import ClassVar

import httpx
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from pydantic import JsonValue
from typing_extensions import override

from .api import ANTHROPIC_VARS, AUTH_HEADERS, EXPLICIT_HEADERS
from .models import PRE_CALL, Scenario

LOGGING_MODULE = "litellm.litellm_core_utils.litellm_logging"


class Capture:
    constructed: ClassVar[dict[str, JsonValue] | None] = None
    pre: ClassVar[dict[str, JsonValue] | None] = None


class RecordingLogging:
    def __init__(self, **kwargs: JsonValue) -> None:
        Capture.constructed = kwargs

    def pre_call(self, **kwargs: JsonValue) -> None:
        call = PRE_CALL.validate_python(kwargs)
        additional = call["additional_args"]
        Capture.pre = {
            "input": call["input"],
            "api_base": additional["api_base"],
            "headers": explicit_headers(additional["headers"]),
            "auth_header": auth_header(additional["headers"]),
            "complete_input_dict": additional["complete_input_dict"],
        }


class FakeLoggingModule(ModuleType):
    Logging: type[RecordingLogging] = RecordingLogging


class BatchClientCache(LLMClientCache):
    def __init__(self, client: AsyncHTTPHandler) -> None:
        super().__init__()
        self.client = client

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> AsyncHTTPHandler:
        return self.client


class Recorder:
    def __init__(self, scenario: Scenario) -> None:
        self.scenario = scenario
        self.seen: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request_view(request))
        return httpx.Response(
            self.scenario.status,
            content=self.scenario.body.encode(),
            headers=self.scenario.headers,
            request=request,
        )


def explicit_headers(headers: dict[str, str]) -> dict[str, JsonValue]:
    return {name: headers[name] for name in EXPLICIT_HEADERS if name in headers}


def plain_headers(headers: dict[str, str]) -> dict[str, JsonValue]:
    return {name: value for name, value in headers.items() if name not in AUTH_HEADERS}


def auth_header(headers: dict[str, str]) -> str | None:
    for name in AUTH_HEADERS:
        if name in headers:
            return name
    return None


def apply_environment(environment: dict[str, str]) -> None:
    for name in ANTHROPIC_VARS:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def request_view(request: httpx.Request) -> JsonValue:
    headers = dict(request.headers)
    return {
        "method": request.method,
        "url": str(request.url),
        "headers": explicit_headers(headers),
        "auth_header": auth_header(headers),
    }


def new_logging(now: int) -> Logging:
    return Logging(
        model="",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime.fromtimestamp(now, tz=UTC),
        litellm_call_id="batch",
        function_id="batch",
    )
