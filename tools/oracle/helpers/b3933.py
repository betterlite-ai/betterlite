import asyncio
import gzip
import json
import os
from collections.abc import Mapping, Sequence
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.integrations.datadog.datadog import DataDogLogger
from litellm.llms.custom_httpx.http_handler import MaskedHTTPStatusError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]

_CLIENT_ERROR_STATUS = 400

_DUMMY_KEYS: tuple[str, ...] = (
    "id",
    "call_type",
    "stream",
    "response_cost",
    "autorouter_savings",
    "response_cost_failure_debug_info",
    "zero_cost_diagnostic",
    "status",
    "total_tokens",
    "prompt_tokens",
    "completion_tokens",
    "startTime",
    "endTime",
    "completionStartTime",
    "model_map_information",
    "model",
    "model_id",
    "model_group",
    "custom_llm_provider",
    "api_base",
    "metadata",
    "cache_hit",
    "cache_key",
    "saved_cache_cost",
    "request_tags",
    "request_model_access_groups",
    "end_user",
    "requester_ip_address",
    "messages",
    "response",
    "error_str",
    "model_parameters",
    "hidden_params",
)


class InputError(ValueError):
    pass


def object_value(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise InputError(f"{name} must be a boolean")
    return value


def plain(value: object) -> Json:
    if isinstance(value, Mapping):
        fields = cast("Mapping[object, object]", value)
        return {str(key): plain(item) for key, item in fields.items()}
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in cast("tuple[object, ...]", value)]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return cast(Json, value)
    raise InputError(f"cannot represent a {type(value).__name__} in a helper answer")


def compact(value: Json) -> Json:
    if isinstance(value, dict):
        return {name: compact(field) for name, field in value.items() if field is not None}
    if isinstance(value, list):
        return [compact(item) for item in value]
    return value


def project(value: Json) -> Json:
    if not isinstance(value, dict):
        return value
    return {key: value[key] for key in _DUMMY_KEYS if key in value}


def envelope(value: Json, action: str) -> Json:
    if not isinstance(value, dict):
        return compact(value)
    compacted = compact(value)
    if not isinstance(compacted, dict):
        return compacted
    message = value.get("message")
    if not isinstance(message, str):
        return compacted
    try:
        parsed = cast(Json, json.loads(message))
    except json.JSONDecodeError:
        return compacted
    compacted["message"] = project(compact(parsed)) if action == "health_check" else compact(parsed)
    return compacted


def normalized_body(value: Json, action: str) -> Json:
    if isinstance(value, list):
        return [envelope(item, action) for item in value]
    return envelope(value, action)


class Script:
    def __init__(self, replies: Sequence[Json]) -> None:
        self.replies: list[Json] = list(replies)
        self.calls: list[Fields] = []
        self.action = ""

    def next_reply(self) -> Fields:
        if not self.replies:
            raise InputError("the scripted transport has no reply")
        return object_value(self.replies.pop(0), "reply")


def record(script: Script, url: str, headers: Mapping[str, str], parsed: Json) -> None:
    lowered = {name.lower(): value for name, value in headers.items()}
    script.calls.append(
        {
            "action": script.action,
            "url": url,
            "headers": cast(Json, lowered),
            "body": normalized_body(parsed, script.action),
        }
    )


def respond(script: Script, url: str, headers: Mapping[str, str], parsed: Json) -> httpx.Response:
    record(script, url, headers, parsed)
    reply = script.next_reply()
    request = httpx.Request("POST", url)
    if reply.get("fail") is True:
        raise httpx.ConnectError("the scripted transport failed", request=request)
    status = integer_value(reply.get("status", 0), "reply.status")
    text = string_value(reply.get("body", ""), "reply.body")
    response = httpx.Response(status_code=status, text=text, request=request)
    if reply.get("raise_status", True) is not False and status >= _CLIENT_ERROR_STATUS:
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as error:
            raise MaskedHTTPStatusError(error, message=text, text=text) from None
    return response


class AsyncHandler:
    def __init__(self, script: Script) -> None:
        self.script = script

    async def post(self, url: str, data: bytes, headers: Mapping[str, str]) -> httpx.Response:
        gzipped = headers.get("content-encoding", "").lower() == "gzip"
        raw = gzip.decompress(data) if gzipped else data
        return respond(self.script, url, headers, cast(Json, json.loads(raw)))


class SyncHandler:
    def __init__(self, script: Script) -> None:
        self.script = script

    def post(self, url: str, json: Json, headers: Mapping[str, str]) -> httpx.Response:
        return respond(self.script, url, headers, json)


class AsyncFactory:
    def __init__(self, handler: AsyncHandler) -> None:
        self.handler = handler

    def __call__(self, llm_provider: str) -> AsyncHandler:  # noqa: ARG002
        return self.handler


class SyncFactory:
    def __init__(self, handler: SyncHandler) -> None:
        self.handler = handler

    def __call__(self) -> SyncHandler:
        return self.handler


async def disable_periodic_flush(logger: DataDogLogger) -> None:
    pass


def apply_environment(values: Mapping[str, str]) -> None:
    cleared = ("HOSTNAME", "POD_NAME", "DATADOG_MOCK", "USE_DDTRACE", "USE_DDPROFILER")
    for name in list(os.environ):
        if name.startswith(("DD_", "LITELLM_DD_")) or name in cleared:
            del os.environ[name]
    os.environ.update(values)


def optional_string(init: Fields, name: str) -> str | None:
    value = init.get(name)
    return None if value is None else string_value(value, f"init.{name}")


def create_logger(init: Fields) -> DataDogLogger:
    allow = init.get("allow_env_credentials")
    return DataDogLogger(
        dd_api_key=optional_string(init, "dd_api_key"),
        dd_site=optional_string(init, "dd_site"),
        dd_agent_host=optional_string(init, "dd_agent_host"),
        dd_agent_port=optional_string(init, "dd_agent_port"),
        allow_env_credentials=(
            True if allow is None else boolean_value(allow, "init.allow_env_credentials")
        ),
    )


def queue_report(logger: DataDogLogger) -> Json:
    payloads = cast("list[Json]", logger.log_queue)
    return {
        "length": len(payloads),
        "head": [envelope(payload, "") for payload in payloads[:3]],
        "tail": [envelope(payload, "") for payload in payloads[-3:]],
    }


async def perform(logger: DataDogLogger, action: Json) -> Json:
    fields = object_value(action, "action")
    step = string_value(fields.get("do"), "action.do")
    if step in ("success", "failure"):
        payload = fields.get("payload")
        kwargs: Json = {"standard_logging_object": payload}
        if step == "success":
            await logger.async_log_success_event(kwargs, None, None, None)  # pyright: ignore[reportUnknownMemberType]
        else:
            await logger.async_log_failure_event(kwargs, None, None, None)  # pyright: ignore[reportUnknownMemberType]
        return {"queue_length": len(cast("list[Json]", logger.log_queue))}
    if step == "flush":
        await logger.flush_queue()
        return {"queue_length": len(cast("list[Json]", logger.log_queue))}
    if step == "send_batch":
        await logger.async_send_batch()
        return {"queue_length": len(cast("list[Json]", logger.log_queue))}
    if step == "health_check":
        return {"health": plain(await logger.async_health_check())}
    if step == "sync_log":
        kwargs = {"standard_logging_object": fields.get("payload")}
        logger.log_success_event(kwargs, {}, None, None)  # pyright: ignore[reportUnknownMemberType]
        return {"queue_length": len(cast("list[Json]", logger.log_queue))}
    if step == "get_payload":
        payload = cast(
            "Json",
            await logger.get_request_response_payload("request-id", None, None),  # pyright: ignore[reportUnknownMemberType]
        )
        return {"payload": plain(payload)}
    raise InputError(f"unknown action: {step}")


async def execute(
    init: Fields,
    script: Script,
    actions: list[Json],
    name: str,
) -> Json:
    try:
        logger = create_logger(init)
    except Exception as error:  # noqa: BLE001 - the helper reports the classified init failure
        message = str(error)
        if "DD_API_KEY" in message:
            return {"name": name, "init": {"error": "missing_api_key"}}
        if "DD_SITE" in message:
            return {"name": name, "init": {"error": "missing_site"}}
        return {"name": name, "init": {"error": "invalid_configuration"}}
    results: list[Json] = []
    for action in actions:
        script.action = string_value(object_value(action, "action").get("do"), "action.do")
        results.append(await perform(logger, action))
    return {
        "name": name,
        "init": {
            "batch_size": logger.batch_size,
            "intake_url": logger.intake_url,
            "authenticated": logger.DD_API_KEY is not None,
            "turn_off_message_logging": bool(logger.turn_off_message_logging),
            "error": None,
        },
        "results": results,
        "calls": cast(Json, script.calls),
        "queue": queue_report(logger),
    }


def run_scenario(scenario: Fields) -> Json:
    name = string_value(scenario.get("name", ""), "scenario.name")
    environment = object_value(scenario.get("environment", {}), "environment")
    values = {
        variable: string_value(value, f"environment.{variable}")
        for variable, value in environment.items()
    }
    init = object_value(scenario.get("init", {}), "init")
    script = Script(list_value(scenario.get("replies", []), "replies"))
    actions = list_value(scenario.get("actions", []), "actions")
    turn_off = init.get("turn_off_message_logging")
    params: Json = (
        None
        if turn_off is None
        else {"turn_off_message_logging": boolean_value(turn_off, "init.turn_off_message_logging")}
    )
    use_v1 = init.get("use_v1")
    apply_environment(values)
    with (
        patch.object(litellm, "datadog_params", params),
        patch.object(litellm, "datadog_use_v1", use_v1 is True),
        patch(
            "litellm.integrations.datadog.datadog.get_async_httpx_client",
            AsyncFactory(AsyncHandler(script)),
        ),
        patch(
            "litellm.integrations.datadog.datadog._get_httpx_client",
            SyncFactory(SyncHandler(script)),
        ),
        patch.object(DataDogLogger, "periodic_flush", disable_periodic_flush),
    ):
        return asyncio.run(execute(init, script, actions, name))


def run(input_value: Fields) -> Json:
    scenarios = list_value(input_value.get("scenarios", []), "scenarios")
    return {
        "scenarios": [run_scenario(object_value(scenario, "scenario")) for scenario in scenarios],
    }
