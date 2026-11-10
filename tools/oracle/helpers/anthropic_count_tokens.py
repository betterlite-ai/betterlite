import asyncio
import json
import os
from collections.abc import Mapping
from typing import Any, Final, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
from litellm.constants import request_timeout as litellm_request_timeout
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
DEFAULT_REPLY: Final = '{"input_tokens":0}'
INTERNAL_SERVER_ERROR: Final = 500


class Reply(TypedDict):
    status: int
    body: str


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
                "url": str(request.url),
                "timeout": timeout["read"],
                "headers": cast(
                    Json,
                    {
                        name.lower(): value
                        for name, value in request.headers.items()
                    },
                ),
                "body": body,
            }
        )
        if self.reply is None:
            raise RuntimeError("fixture transport failure")
        return httpx.Response(
            self.reply["status"],
            content=self.reply["body"].encode(),
            request=request,
        )

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


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return cast(Object, value)


def text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    return None if value is None else text(value, name)


def number(fields: Object, name: str) -> float | None:
    value = fields.get(name)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} must be a number")
    return float(value)


def objects(fields: Object, name: str) -> list[Object]:
    value = fields.get(name, [])
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(item, f"{name} entry") for item in value]


def strings(fields: Object, name: str) -> list[str]:
    value = fields.get(name, [])
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [text(item, f"{name} entry") for item in value]


def providers(fields: Object) -> list[str | None]:
    value = fields.get("providers", [])
    if not isinstance(value, list):
        raise TypeError("providers must be a list")
    found: list[str | None] = []
    for item in value:
        if item is None or isinstance(item, str):
            found.append(item)
        else:
            raise TypeError("providers entries must be strings or null")
    return found


def environment(fields: Object) -> dict[str, str]:
    value = fields.get("environment")
    if value is None:
        return {}
    raw = object_value(value, "environment")
    found: dict[str, str] = {}
    for name, item in raw.items():
        found[name] = text(item, f"environment.{name}")
    return found


def reply_of(fields: Object) -> Reply | None:
    if fields.get("failure", False) is True:
        return None
    value = fields.get("reply")
    if value is None:
        return {"status": 200, "body": DEFAULT_REPLY}
    raw = object_value(value, "reply")
    status = raw.get("status", 200)
    if isinstance(status, bool) or not isinstance(status, int):
        raise TypeError("reply.status must be a number")
    return {"status": status, "body": text(raw.get("body"), "reply.body")}


def request_timeout(override: Json) -> float:
    if override is None:
        return float(litellm_request_timeout)
    if isinstance(override, bool) or not isinstance(override, (int, float)):
        raise TypeError("request_timeout must be a number")
    return float(override)


class Logger:
    def __init__(self, warnings: Warnings) -> None:
        self.warnings = warnings

    def warning(self, message: str, *args: object) -> None:
        self.warnings.warning(message, *args)


def messages_of(fields: Object) -> list[dict[str, JsonValue]] | None:
    value = fields.get("messages")
    return None if value is None else cast("list[dict[str, JsonValue]]", value)


def mandatory_messages_of(fields: Object) -> list[dict[str, JsonValue]]:
    return cast("list[dict[str, JsonValue]]", fields.get("messages"))


def tools_of(fields: Object) -> list[dict[str, JsonValue]] | None:
    value = fields.get("tools")
    return None if value is None else cast("list[dict[str, JsonValue]]", value)


def system_of(fields: Object) -> JsonValue:
    return cast(JsonValue, fields.get("system"))


def optional_params_of(fields: Object) -> Mapping[str, JsonValue] | None:
    value = fields.get("optional_params")
    return None if value is None else cast("Mapping[str, JsonValue]", value)


def observed(call: Object) -> Json:
    headers = object_value(call["headers"], "headers")
    return {
        "url": call["url"],
        "timeout": call["timeout"],
        "headers": cast(
            Json, {name: headers.get(name) for name in COUNT_HEADERS}
        ),
        "body": call["body"],
    }


def error_message_kind(message: str | None, status: int | None) -> str | None:
    if message is None:
        return None
    if status == INTERNAL_SERVER_ERROR:
        return (
            "processing"
            if message.startswith("CountTokens processing error: ")
            else "generic"
        )
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


def validate(cases: list[Object]) -> Json:
    config = AnthropicCountTokensConfig()
    results: list[Json] = []
    for case in cases:
        try:
            config.validate_request(
                model=cast(str, case.get("model")),
                messages=mandatory_messages_of(case),
                system=system_of(case),
                tools=tools_of(case),
            )
        except ValueError as error:
            results.append({"ok": False, "message": str(error)})
        else:
            results.append({"ok": True, "message": None})
    return {"results": results}


def transform(cases: list[Object]) -> Json:
    config = AnthropicCountTokensConfig()
    bodies: list[Json] = []
    for case in cases:
        body = config.transform_request_to_count_tokens(
            model=cast(str, case.get("model")),
            messages=mandatory_messages_of(case),
            tools=tools_of(case),
            system=system_of(case),
            optional_params=optional_params_of(case),
        )
        bodies.append(cast(Json, body))
    return {"bodies": bodies}


def headers(cases: list[Object]) -> Json:
    config = AnthropicCountTokensConfig()
    answers: list[Json] = []
    for case in cases:
        raw = config.get_required_headers(cast(str, case.get("api_key")))
        normalized = {name.lower(): value for name, value in raw.items()}
        answers.append(cast(Json, {name: normalized.get(name) for name in COUNT_HEADERS}))
    return {"headers": answers}


def handle_case(fields: Object) -> Json:
    provider = Provider(reply_of(fields))
    handler = AnthropicCountTokensHandler()
    timeout = number(fields, "timeout")
    request_timeout_value = request_timeout(fields.get("request_timeout"))
    with (
        mock.patch.dict(os.environ, environment(fields), clear=True),
        mock.patch(HANDLER_CLIENT, return_value=provider.client()),
        mock.patch("litellm.request_timeout", request_timeout_value),
    ):
        try:
            response = asyncio.run(
                handler.handle_count_tokens_request(
                    model=cast(str, fields.get("model")),
                    messages=mandatory_messages_of(fields),
                    api_key=cast(str, fields.get("api_key")),
                    api_base=optional_text(fields, "api_base"),
                    timeout=timeout,
                    tools=tools_of(fields),
                    system=system_of(fields),
                    optional_params=optional_params_of(fields),
                )
            )
        except AnthropicError as error:
            call = provider.calls[0] if provider.calls else None
            return {
                "request": None if call is None else observed(call),
                "response": {
                    "error": {
                        "status_code": error.status_code,
                        "processing": str(error.message).startswith(
                            "CountTokens processing error: "
                        ),
                    }
                },
            }
    call = provider.calls[0] if provider.calls else None
    return {
        "request": None if call is None else observed(call),
        "response": {"value": cast(Json, response)},
    }


def handle(cases: list[Object]) -> Json:
    return {"calls": [handle_case(case) for case in cases]}


def counter_case(fields: Object) -> Json:
    provider = Provider(reply_of(fields))
    counter = AnthropicTokenCounter()
    warnings = Warnings()
    request_timeout_value = request_timeout(fields.get("request_timeout"))
    with (
        mock.patch.dict(os.environ, environment(fields), clear=True),
        mock.patch(HANDLER_CLIENT, return_value=provider.client()),
        mock.patch("litellm.request_timeout", request_timeout_value),
        mock.patch(COUNTER_LOGGER, new=Logger(warnings)),
        mock.patch.object(litellm, "api_key", optional_text(fields, "module_api_key")),
        mock.patch.object(
            litellm,
            "anthropic_key",
            optional_text(fields, "module_anthropic_key"),
        ),
    ):
        result = asyncio.run(
            counter.count_tokens(
                model_to_use=cast(str, fields.get("model")),
                messages=cast("list[dict[str, Json]] | None", fields.get("messages")),
                contents=cast("list[dict[str, Json]] | None", fields.get("contents")),
                deployment=cast("dict[str, Any] | None", fields.get("deployment")),
                request_model=optional_text(fields, "request_model") or "",
                tools=cast("list[dict[str, Json]] | None", fields.get("tools")),
                system=fields.get("system"),
            )
        )
    selected = counter.should_use_token_counting_api(optional_text(fields, "provider"))
    call = provider.calls[0] if provider.calls else None
    return {
        "selected": selected,
        "result": None if result is None else result_json(result),
        "request": None if call is None else observed(call),
        "warnings": cast(Json, warnings.entries),
    }


def counter(cases: list[Object]) -> Json:
    return {"calls": [counter_case(case) for case in cases]}


def factory_case(fields: Object) -> Json:
    provider = Provider(reply_of(fields))
    counter = AnthropicModelInfo().get_token_counter()
    if not isinstance(counter, AnthropicTokenCounter):
        raise TypeError("the Anthropic model info did not return the native token counter")
    models = strings(fields, "models")
    deployment = cast("dict[str, Any] | None", fields.get("deployment"))
    selected: list[Json] = [
        counter.should_use_token_counting_api(value) for value in providers(fields)
    ]
    results: list[Json] = []
    with (
        mock.patch.dict(os.environ, environment(fields), clear=True),
        mock.patch(HANDLER_CLIENT, return_value=provider.client()),
        mock.patch("litellm.request_timeout", request_timeout(None)),
    ):
        for model in models:
            outcome = asyncio.run(
                counter.count_tokens(
                    model_to_use=model,
                    messages=cast("list[dict[str, Json]] | None", fields.get("messages")),
                    contents=None,
                    deployment=deployment,
                    request_model="",
                    tools=None,
                    system=None,
                )
            )
            results.append(None if outcome is None else result_json(outcome))
    return {
        "selected": selected,
        "results": results,
        "urls": [call["url"] for call in provider.calls],
    }


def factory(cases: list[Object]) -> Json:
    return {"cases": [factory_case(case) for case in cases]}


def run(argument: dict[str, Json]) -> Json:
    [(function, body)] = argument.items()
    fields = object_value(body, "input")
    cases = objects(fields, "cases")
    match function:
        case "validate":
            return validate(cases)
        case "transform":
            return transform(cases)
        case "headers":
            return headers(cases)
        case "handle":
            return handle(cases)
        case "counter":
            return counter(cases)
        case "factory":
            return factory(cases)
        case _:
            raise ValueError(f"unsupported function {function}")
