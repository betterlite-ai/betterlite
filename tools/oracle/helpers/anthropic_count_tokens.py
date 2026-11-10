import asyncio
import json
import os
from contextlib import ExitStack
from typing import Final, NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
from litellm.constants import request_timeout as default_timeout
from litellm.llms.anthropic.common_utils import AnthropicError, AnthropicModelInfo
from litellm.llms.anthropic.count_tokens.handler import AnthropicCountTokensHandler
from litellm.llms.anthropic.count_tokens.token_counter import AnthropicTokenCounter
from litellm.llms.anthropic.count_tokens.transformation import AnthropicCountTokensConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import TokenCountResponse
from pydantic import JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

HANDLER_CLIENT: Final = "litellm.llms.anthropic.count_tokens.handler.get_async_httpx_client"
COUNTER_LOGGER: Final = "litellm.llms.anthropic.count_tokens.token_counter.verbose_logger"
COUNT_HEADERS: Final = (
    "content-type",
    "x-api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
    "anthropic-dangerous-direct-browser-access",
)
INTERNAL_SERVER_ERROR: Final = 500

Messages: TypeAlias = list[dict[str, JsonValue]]


class Reply(TypedDict):
    status: NotRequired[int]
    body: str


class Case(TypedDict, total=False):
    model: str
    messages: Messages
    contents: Messages
    system: JsonValue
    tools: Messages
    optional_params: dict[str, JsonValue]
    api_key: str
    api_base: str
    timeout: float
    request_timeout: float
    deployment: dict[str, JsonValue]
    request_model: str
    provider: str
    environment: dict[str, str]
    module_api_key: str
    module_anthropic_key: str
    reply: Reply
    models: list[str]
    providers: list[str | None]


class Provider:
    def __init__(self, reply: Reply | None) -> None:
        self.reply = reply
        self.calls: list[Object] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        timeout = cast(dict[str, float], request.extensions["timeout"])
        content = request.content
        body: Json = None if not content else cast(Json, json.loads(content))
        self.calls.append(
            {
                "url": f"{request.url.scheme}://{request.url.netloc.decode()}{request.url.raw_path.decode()}",
                "timeout": float(timeout["read"]),
                "headers": {name.lower(): value for name, value in request.headers.items()},
                "body": body,
            }
        )
        if self.reply is None:
            raise RuntimeError("fixture transport failure")
        return httpx.Response(
            self.reply.get("status", 200),
            content=self.reply["body"].encode(),
            request=request,
        )

    def observed(self) -> Json:
        if not self.calls:
            return None
        call = self.calls[0]
        sent = cast(Object, call["headers"])
        headers = {name: sent.get(name) for name in COUNT_HEADERS}
        return {
            "url": call["url"],
            "timeout": call["timeout"],
            "headers": headers,
            "body": call["body"],
        }

    def client(self) -> AsyncHTTPHandler:
        handler = AsyncHTTPHandler()
        handler.client = httpx.AsyncClient(transport=httpx.MockTransport(self.handle))
        return handler


class Warnings:
    def __init__(self) -> None:
        self.entries: list[Json] = []

    def warning(self, message: str, *args: object) -> None:
        rendered = message % args if args else message
        subject, _separator, detail = rendered.partition(": ")
        status: Json = None
        if detail.startswith("status="):
            head = detail.removeprefix("status=").split(",", 1)[0]
            try:
                status = int(head)
            except ValueError:
                status = None
        self.entries.append({"subject": subject, "status": status})


def error_message_kind(message: str | None, status: int | None) -> str | None:
    if message is None:
        return None
    if status == INTERNAL_SERVER_ERROR:
        return "processing" if message.startswith("CountTokens processing error: ") else "generic"
    return message


def result_json(result: TokenCountResponse) -> Json:
    return {
        "total_tokens": result.total_tokens,
        "request_model": result.request_model,
        "model_used": result.model_used,
        "tokenizer_type": result.tokenizer_type,
        "original_response": cast(Json, result.original_response),
        "error": result.error,
        "error_message": error_message_kind(result.error_message, result.status_code),
        "status_code": result.status_code,
    }


def validate(cases: list[Case]) -> Json:
    config = AnthropicCountTokensConfig()
    results: list[Json] = []
    for case in cases:
        try:
            config.validate_request(
                model=case.get("model", ""),
                messages=cast(Messages, case.get("messages")),
                system=case.get("system"),
                tools=case.get("tools"),
            )
        except ValueError as error:
            results.append({"ok": False, "message": str(error)})
        else:
            results.append({"ok": True, "message": None})
    return {"results": results}


def transform(cases: list[Case]) -> Json:
    config = AnthropicCountTokensConfig()
    bodies: list[Json] = []
    for case in cases:
        body = config.transform_request_to_count_tokens(
            model=case.get("model", ""),
            messages=cast(Messages, case.get("messages")),
            tools=case.get("tools"),
            system=case.get("system"),
            optional_params=case.get("optional_params"),
        )
        bodies.append(cast(Json, body))
    return {"bodies": bodies}


def headers(cases: list[Case]) -> Json:
    config = AnthropicCountTokensConfig()
    answers: list[Json] = []
    for case in cases:
        raw = config.get_required_headers(case.get("api_key", ""))
        normalized = {name.lower(): value for name, value in raw.items()}
        answers.append(cast(Json, {name: normalized.get(name) for name in COUNT_HEADERS}))
    return {"headers": answers}


def patched(case: Case, provider: Provider) -> ExitStack:
    stack = ExitStack()
    stack.enter_context(mock.patch.dict(os.environ, case.get("environment", {}), clear=True))
    stack.enter_context(mock.patch(HANDLER_CLIENT, return_value=provider.client()))
    stack.enter_context(
        mock.patch("litellm.request_timeout", float(case.get("request_timeout", default_timeout)))
    )
    return stack


def handle_case(case: Case) -> Json:
    provider = Provider(case.get("reply"))
    handler = AnthropicCountTokensHandler()
    with patched(case, provider):
        try:
            response = asyncio.run(
                handler.handle_count_tokens_request(
                    model=case.get("model", ""),
                    messages=cast(Messages, case.get("messages")),
                    api_key=case.get("api_key", ""),
                    api_base=case.get("api_base"),
                    timeout=case.get("timeout"),
                    tools=case.get("tools"),
                    system=case.get("system"),
                    optional_params=case.get("optional_params"),
                )
            )
        except AnthropicError as error:
            processing = str(error.message).startswith("CountTokens processing error: ")
            failure: Json = {"error": {"status_code": error.status_code, "processing": processing}}
            return {"request": provider.observed(), "response": failure}
    return {"request": provider.observed(), "response": {"value": cast(Json, response)}}


def counter_case(case: Case) -> Json:
    provider = Provider(case.get("reply"))
    counter = AnthropicTokenCounter()
    warnings = Warnings()
    with (
        patched(case, provider),
        mock.patch(COUNTER_LOGGER, new=warnings),
        mock.patch.object(litellm, "api_key", case.get("module_api_key")),
        mock.patch.object(litellm, "anthropic_key", case.get("module_anthropic_key")),
    ):
        result = asyncio.run(
            counter.count_tokens(
                model_to_use=case.get("model", ""),
                messages=case.get("messages"),
                contents=case.get("contents"),
                deployment=case.get("deployment"),
                request_model=case.get("request_model", ""),
                tools=case.get("tools"),
                system=case.get("system"),
            )
        )
    return {
        "selected": counter.should_use_token_counting_api(case.get("provider")),
        "result": None if result is None else result_json(result),
        "request": provider.observed(),
        "warnings": cast(Json, warnings.entries),
    }


def factory_case(case: Case) -> Json:
    provider = Provider(case.get("reply"))
    counter = AnthropicModelInfo().get_token_counter()
    if not isinstance(counter, AnthropicTokenCounter):
        raise TypeError("the Anthropic model info did not return the native token counter")
    selected: list[Json] = [
        counter.should_use_token_counting_api(value) for value in case.get("providers", [])
    ]
    results: list[Json] = []
    with patched({"environment": case.get("environment", {})}, provider):
        for model in case.get("models", []):
            outcome = asyncio.run(
                counter.count_tokens(
                    model_to_use=model,
                    messages=case.get("messages"),
                    contents=None,
                    deployment=case.get("deployment"),
                )
            )
            results.append(None if outcome is None else result_json(outcome))
    return {
        "selected": selected,
        "results": results,
        "urls": [call["url"] for call in provider.calls],
    }


def run(argument: dict[str, Json]) -> Json:
    function = argument.get("function")
    cases = cast("list[Case]", argument.get("cases", []))
    match function:
        case "validate":
            return validate(cases)
        case "transform":
            return transform(cases)
        case "headers":
            return headers(cases)
        case "handle":
            return {"calls": [handle_case(case) for case in cases]}
        case "counter":
            return {"calls": [counter_case(case) for case in cases]}
        case "factory":
            return {"cases": [factory_case(case) for case in cases]}
        case _:
            raise ValueError(f"unsupported function {function}")
