import asyncio
import gzip
import json
import os
from collections.abc import Mapping
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
from litellm.integrations.datadog.datadog import DataDogLogger
from litellm.llms.custom_httpx.http_handler import MaskedHTTPStatusError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_CLIENT_ERROR_STATUS = 400


class Reply(TypedDict, total=False):
    status: int
    body: str
    raise_status: bool


class Action(TypedDict):
    do: str
    payload: NotRequired[Json]


class Scenario(TypedDict, total=False):
    name: str
    environment: dict[str, str]
    init: dict[str, Json]
    replies: list[Reply]
    actions: list[Action]


def compact(value: Json) -> Json:
    if isinstance(value, dict):
        return {name: compact(field) for name, field in value.items() if field is not None}
    if isinstance(value, list):
        return [compact(item) for item in value]
    return value


def envelope(value: Json) -> Json:
    compacted = compact(value)
    if not isinstance(value, dict) or not isinstance(compacted, dict):
        return compacted
    message = value.get("message")
    if isinstance(message, str):
        compacted["message"] = compact(cast(Json, json.loads(message)))
    return compacted


class Script:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = replies
        self.calls: list[Json] = []

    def respond(self, url: str, headers: Mapping[str, str], parsed: Json) -> httpx.Response:
        body = [envelope(item) for item in parsed] if isinstance(parsed, list) else envelope(parsed)
        lowered = {name.lower(): value for name, value in headers.items()}
        self.calls.append(cast(Json, {"url": url, "headers": lowered, "body": body}))
        reply = self.replies.pop(0)
        status = reply.get("status", 0)
        text = reply.get("body", "")
        response = httpx.Response(status, text=text, request=httpx.Request("POST", url))
        if reply.get("raise_status", True) and status >= _CLIENT_ERROR_STATUS:
            try:
                response.raise_for_status()
            except httpx.HTTPStatusError as error:
                raise MaskedHTTPStatusError(error, message=text, text=text) from None
        return response

    def __call__(self, llm_provider: str) -> "Script":  # noqa: ARG002
        return self

    async def post(self, url: str, data: bytes, headers: Mapping[str, str]) -> httpx.Response:
        lowered = {name.lower(): value for name, value in headers.items()}
        gzipped = lowered.get("content-encoding", "").lower() == "gzip"
        raw = gzip.decompress(data) if gzipped else data
        return self.respond(url, headers, cast(Json, json.loads(raw)))


class SyncScript:
    def __init__(self, script: Script) -> None:
        self.script = script

    def __call__(self) -> "SyncScript":
        return self

    def post(self, url: str, json: Json, headers: Mapping[str, str]) -> httpx.Response:
        return self.script.respond(url, {"Content-Type": "application/json", **headers}, json)


async def disable_periodic_flush(logger: DataDogLogger) -> None:
    pass


def queue_length(logger: DataDogLogger) -> int:
    return len(cast("list[Json]", logger.log_queue))


async def perform(logger: DataDogLogger, action: Action) -> Json:
    kwargs: Json = {"standard_logging_object": action.get("payload")}
    step = action["do"]
    if step == "success":
        await logger.async_log_success_event(kwargs, None, None, None)  # pyright: ignore[reportUnknownMemberType]
    elif step == "failure":
        await logger.async_log_failure_event(kwargs, None, None, None)  # pyright: ignore[reportUnknownMemberType]
    elif step == "flush":
        await logger.flush_queue()
    elif step == "sync_log":
        logger.log_success_event(kwargs, {}, None, None)  # pyright: ignore[reportUnknownMemberType]
    else:
        status = await logger.async_health_check()
        return {"health": cast(Json, dict(status))}
    return {"queue_length": queue_length(logger)}


async def execute(scenario: Scenario, script: Script) -> Json:
    name = scenario.get("name", "")
    init = scenario.get("init", {})
    allow = init.get("allow_env_credentials")
    try:
        logger = DataDogLogger(
            dd_api_key=cast("str | None", init.get("dd_api_key")),
            dd_site=cast("str | None", init.get("dd_site")),
            dd_agent_host=cast("str | None", init.get("dd_agent_host")),
            dd_agent_port=cast("str | None", init.get("dd_agent_port")),
            allow_env_credentials=True if allow is None else cast(bool, allow),
        )
    except Exception as error:  # noqa: BLE001 - the helper reports which setting was missing
        missing = "missing_api_key" if "DD_API_KEY" in str(error) else "missing_site"
        return {"name": name, "init": {"error": missing}}
    results = [await perform(logger, action) for action in scenario.get("actions", [])]
    queued = cast("list[Json]", logger.log_queue)
    return {
        "name": name,
        "init": {
            "batch_size": logger.batch_size,
            "intake_url": logger.intake_url,
            "authenticated": logger.DD_API_KEY is not None,
            "error": None,
        },
        "results": results,
        "calls": script.calls,
        "queue": {
            "length": len(queued),
            "head": [envelope(payload) for payload in queued[:3]],
            "tail": [envelope(payload) for payload in queued[-3:]],
        },
    }


def run_scenario(scenario: Scenario) -> Json:
    script = Script(scenario.get("replies", []))
    cleared = ("HOSTNAME", "POD_NAME", "DATADOG_MOCK", "USE_DDTRACE", "USE_DDPROFILER")
    for name in list(os.environ):
        if name.startswith(("DD_", "LITELLM_DD_")) or name in cleared:
            del os.environ[name]
    os.environ.update(scenario.get("environment", {}))
    with (
        patch("litellm.integrations.datadog.datadog.get_async_httpx_client", script),
        patch("litellm.integrations.datadog.datadog._get_httpx_client", SyncScript(script)),
        patch.object(DataDogLogger, "periodic_flush", disable_periodic_flush),
    ):
        return asyncio.run(execute(scenario, script))


def run(input_value: dict[str, list[Scenario]]) -> Json:
    return {"scenarios": [run_scenario(scenario) for scenario in input_value["scenarios"]]}
