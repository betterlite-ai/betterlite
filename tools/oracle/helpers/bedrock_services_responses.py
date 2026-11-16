import asyncio
import logging
import os
from base64 import b64encode
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict
from unittest.mock import Mock, patch

import botocore.auth
import httpx
from botocore.credentials import Credentials
from litellm.exceptions import ImageFetchError
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.responses.transformation import (
    BedrockOpenAIResponsesConfig,
    collect_remote_image_urls,
    inline_remote_image_urls,
)
from litellm.types.llms.openai import ResponseInputParam, ResponsesAPIOptionalRequestParams
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Object: TypeAlias = dict[str, JsonValue]
Input: TypeAlias = str | ResponseInputParam

JSON: Final = TypeAdapter(JsonValue)
MODEL: Final = "openai.gpt-oss-120b"
API_BASE: Final = "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1/responses"
ENVIRONMENT_KEYS: Final = (
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_BEDROCK_RUNTIME_ENDPOINT",
    "AWS_REGION",
    "AWS_REGION_NAME",
)


class RouteCase(TypedDict):
    model: str | None
    model_cost: NotRequired[Object]


class AuthCase(TypedDict):
    api_key: NotRequired[str]
    headers: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]
    sign: NotRequired[bool]


class ErrorProbe(TypedDict):
    status: int
    message: str
    headers: dict[str, str]


class SigningCase(TypedDict):
    access_key_id: str
    secret_access_key: str
    headers: dict[str, str]
    optional_params: Object
    request_data: Object
    now: int
    error: NotRequired[ErrorProbe]


class OptionCase(TypedDict):
    params: ResponsesAPIOptionalRequestParams
    drop_params: NotRequired[bool]


class InputCase(TypedDict):
    input: Input


class ReplacementCase(InputCase):
    inlined: dict[str, str]


class FetchCase(InputCase):
    data_urls: dict[str, str]


class CodexCase(InputCase):
    model: str
    params: NotRequired[Object]


class UrlCase(TypedDict):
    api_base: NotRequired[str]
    litellm_params: NotRequired[Object]
    environment: NotRequired[dict[str, str]]


class Calls(Protocol):
    def validate_environment(
        self, headers: Object, model: str, litellm_params: GenericLiteLLMParams | None, /
    ) -> Object: ...

    def sign_request(
        self,
        headers: Object,
        optional_params: Object,
        request_data: Object,
        api_base: str,
        /,
        *,
        api_key: str | None = None,
    ) -> tuple[Object, bytes | None]: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, object] | httpx.Headers, /
    ) -> BaseLLMException: ...

    def map_openai_params(
        self,
        response_api_optional_params: ResponsesAPIOptionalRequestParams,
        model: str,
        /,
        *,
        drop_params: bool,
    ) -> Object: ...

    def get_complete_url(self, api_base: str | None, litellm_params: Object, /) -> str: ...

    def transform_responses_api_request(
        self,
        model: str,
        source: Input,
        params: Object,
        litellm_params: GenericLiteLLMParams,
        headers: Object,
        /,
    ) -> Object: ...

    async def async_transform_responses_api_request(
        self,
        model: str,
        source: Input,
        params: Object,
        litellm_params: GenericLiteLLMParams,
        headers: Object,
        /,
    ) -> Object: ...

    def supports_native_file_search(self) -> bool: ...


class Fixture:
    def __init__(self, data_urls: dict[str, str]) -> None:
        self.data_urls = data_urls
        self.fetched: list[JsonValue] = []

    def fetch(self, url: str) -> str:
        self.fetched.append(url)
        data_url = self.data_urls.get(url)
        if data_url is None:
            raise ImageFetchError(message=f"no fixture for {url}")
        return data_url

    async def fetch_async(self, url: str) -> str:
        return self.fetch(url)


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


@contextmanager
def warnings_recorded() -> Generator[list[JsonValue]]:
    recorder = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(recorder)
    try:
        yield recorder.messages
    finally:
        logger.removeHandler(recorder)


@contextmanager
def environment(values: dict[str, str] | None) -> Generator[None]:
    given = {} if values is None else values
    kept = {name: value for name, value in os.environ.items() if name not in ENVIRONMENT_KEYS}
    with (
        patch.dict(os.environ, {**kept, **given}, clear=True),
        patch("boto3.Session", Mock(return_value=SimpleNamespace(region_name=None))),
    ):
        yield


def route_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[RouteCase]).validate_python(payload):
        cost = case.get("model_cost")
        with patch.dict("litellm.model_cost", {} if cost is None else cost):
            config = BedrockOpenAIResponsesConfig.for_model(case["model"])
        if config is None:
            results.append({"selected": False, "provider": None, "native_file_search": None})
        else:
            results.append(
                {
                    "selected": True,
                    "provider": config.custom_llm_provider.value,
                    "native_file_search": config.supports_native_file_search(),
                }
            )
    return {"results": results}


def auth_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[AuthCase]).validate_python(payload):
        api_key = case.get("api_key")
        given = case.get("headers")
        headers: Object = {} if given is None else {**given}
        with environment(case.get("environment")):
            params = GenericLiteLLMParams(api_key=api_key)
            validated = calls.validate_environment(headers, MODEL, params)
            signed: JsonValue = None
            if case.get("sign") is True:
                signed_headers, body = calls.sign_request(
                    headers, {}, {}, API_BASE, api_key=api_key
                )
                signed = {"headers": signed_headers, "body_is_none": body is None}
        results.append({"headers": validated, "signed": signed})
    return {"results": results}


def sigv4_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[SigningCase]).validate_python(payload):
        credentials = Credentials(case["access_key_id"], case["secret_access_key"])
        frozen = datetime.fromtimestamp(case["now"], tz=UTC).replace(tzinfo=None)
        with (
            environment(None),
            patch.object(BaseAWSLLM, "resolve_credentials", Mock(return_value=credentials)),
            patch.object(botocore.auth, "get_current_datetime", Mock(return_value=frozen)),
        ):
            headers, body = calls.sign_request(
                case["headers"].copy(), case["optional_params"], case["request_data"], API_BASE
            )
        probe = case.get("error")
        error: JsonValue = None
        if probe is not None:
            raised = calls.get_error_class(probe["message"], probe["status"], probe["headers"])
            error = {
                "status": raised.status_code,
                "message": raised.message,
                "request_id": raised.response.headers.get("x-amzn-requestid"),
            }
        encoded = None if body is None else b64encode(body).decode()
        results.append({"headers": headers, "body": encoded, "error": error})
    return {"results": results}


def option_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[OptionCase]).validate_python(payload):
        with warnings_recorded() as messages:
            drop_params = case.get("drop_params") is True
            mapped = calls.map_openai_params(case["params"], MODEL, drop_params=drop_params)
        native = calls.supports_native_file_search()
        results.append({"params": mapped, "warnings": messages, "native_file_search": native})
    return {"results": results}


def traversal_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[InputCase]).validate_python(payload)
    results: list[JsonValue] = [[*collect_remote_image_urls(case["input"])] for case in cases]
    return {"results": results}


def replacement_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[ReplacementCase]).validate_python(payload):
        try:
            replaced = inline_remote_image_urls(case["input"], case["inlined"])
        except KeyError:
            results.append({"error": True})
        else:
            results.append({"input": JSON.validate_python(replaced)})
    return {"results": results}


def fetch_outcome(case: FetchCase, *, asynchronous: bool) -> JsonValue:
    fixture = Fixture(case["data_urls"])
    config = BedrockOpenAIResponsesConfig(
        fetch_image=fixture.fetch, async_fetch_image=fixture.fetch_async
    )
    calls: Calls = config
    params = GenericLiteLLMParams()
    try:
        if asynchronous:
            body = asyncio.run(
                calls.async_transform_responses_api_request(MODEL, case["input"], {}, params, {})
            )
        else:
            body = calls.transform_responses_api_request(MODEL, case["input"], {}, params, {})
    except ImageFetchError as error:
        return {"error": {"status": error.status_code}, "fetched": [*fixture.fetched]}
    return {"body": body, "fetched": [*fixture.fetched]}


def fetch_cases(_calls: Calls, payload: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[FetchCase]).validate_python(payload)
    results: list[JsonValue] = [
        {
            "sync": fetch_outcome(case, asynchronous=False),
            "async": fetch_outcome(case, asynchronous=True),
        }
        for case in cases
    ]
    return {"results": results}


def url_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[UrlCase]).validate_python(payload):
        params = case.get("litellm_params")
        with environment(case.get("environment")):
            try:
                url = calls.get_complete_url(case.get("api_base"), {} if params is None else params)
            except ValueError:
                results.append({"error": True})
            else:
                results.append({"url": url})
    return {"results": results}


def codex_cases(calls: Calls, payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in TypeAdapter(list[CodexCase]).validate_python(payload):
        params = case.get("params")
        given = {} if params is None else params
        with warnings_recorded() as messages:
            body = calls.transform_responses_api_request(
                case["model"], case["input"], given, GenericLiteLLMParams(), {}
            )
        results.append({"body": body, "warnings": messages})
    return {"results": results}


HANDLERS: Final[dict[str, Callable[[Calls, JsonValue], JsonValue]]] = {
    "bedrock_responses_model_route": route_cases,
    "responses_bearer_auth": auth_cases,
    "responses_sigv4": sigv4_cases,
    "response_option_gates": option_cases,
    "native_file_search_flag": option_cases,
    "inline_image_traversal": traversal_cases,
    "remote_url_inlined": fetch_cases,
    "responses_runtime_url": url_cases,
    "inline_image_replacement": replacement_cases,
    "codex_input_rewrite": codex_cases,
}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    if len(input_value) != 1:
        raise ValueError("bedrock_services_responses takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    # LiteLLM reads LITELLM_RUST per call; false pins the Python fallback to compare against
    os.environ["LITELLM_RUST"] = "false"
    return handler(BedrockOpenAIResponsesConfig(), payload)
