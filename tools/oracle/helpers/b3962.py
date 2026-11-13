import asyncio
import os
from collections.abc import Awaitable, Callable
from datetime import datetime
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.integrations.braintrust_logging import BraintrustLogger

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SUCCESS_STATUSES = range(200, 400)
ResponseObject: TypeAlias = (
    litellm.ModelResponse
    | litellm.TextCompletionResponse
    | litellm.EmbeddingResponse
    | litellm.ImageResponse
)


def apply_environment(scenario: dict[str, Json]) -> None:
    # every helper call is a fresh process, so the scenario replaces the inherited environment
    os.environ.clear()
    environment = scenario.get("environment")
    if not isinstance(environment, dict):
        return
    for name, value in environment.items():
        if isinstance(value, str):
            os.environ[name] = value


class Recorder:
    def __init__(self, answers: list[Json]) -> None:
        self.requests: list[Json] = []
        self.answers = answers

    def record(self, url: str, headers: dict[str, Json] | None, body: Json) -> Json:
        normalized: dict[str, Json] = {
            name.lower(): value for name, value in (headers or {}).items()
        }
        self.requests.append({"url": str(url), "headers": normalized, "body": body})
        return self.answers.pop(0) if self.answers else {}


def reply(recorded: Json, url: str) -> httpx.Response:
    answer = recorded if isinstance(recorded, dict) else {}
    status = answer.get("status")
    status_code = status if isinstance(status, int) and not isinstance(status, bool) else 200
    response = httpx.Response(status_code, json=answer.get("body"))
    if status_code not in SUCCESS_STATUSES:
        raise httpx.HTTPStatusError(
            f"status {status_code}",
            request=httpx.Request("POST", url),
            response=response,
        )
    return response


class FakeSyncClient:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    def post(
        self,
        url: str,
        headers: dict[str, Json] | None = None,
        json: Json = None,
    ) -> httpx.Response:
        return reply(self.recorder.record(url, headers, json), url)


class FakeAsyncClient:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder

    async def post(
        self,
        url: str,
        headers: dict[str, Json] | None = None,
        json: Json = None,
    ) -> httpx.Response:
        return reply(self.recorder.record(url, headers, json), url)


def build_logger(scenario: dict[str, Json]) -> BraintrustLogger:
    apply_environment(scenario)
    key = scenario.get("api_key")
    base = scenario.get("api_base")
    return BraintrustLogger(
        api_key=key if isinstance(key, str) else None,
        api_base=base if isinstance(base, str) else None,
    )


def resolve_sync(logger: BraintrustLogger, name: str) -> str:
    return logger.get_project_id_sync(name)


async def resolve_async(logger: BraintrustLogger, name: str) -> str:
    return await logger.get_project_id_async(name)


def project_result(scenario: dict[str, Json]) -> Json:
    logger = build_logger(scenario)
    answers = scenario.get("answers")
    recorder = Recorder(list(answers) if isinstance(answers, list) else [])
    name = scenario.get("project_name")
    if not isinstance(name, str):
        raise ValueError("project_name must be a string")
    calls = scenario.get("calls")
    count = calls if isinstance(calls, int) and not isinstance(calls, bool) else 1
    asynchronous = scenario.get("path") == "async"
    ids: list[Json] = []
    with (
        patch.object(logger, "global_braintrust_sync_http_handler", FakeSyncClient(recorder)),
        patch.object(logger, "global_braintrust_http_handler", FakeAsyncClient(recorder)),
    ):
        for _ in range(count):
            if asynchronous:
                ids.append(asyncio.run(resolve_async(logger, name)))
            else:
                ids.append(resolve_sync(logger, name))
    return cast(Json, {"requests": recorder.requests, "project_ids": ids})


def build_response(value: Json) -> ResponseObject | None:
    if not isinstance(value, dict):
        return None
    body = value.get("body")
    if not isinstance(body, dict):
        raise ValueError("response.body must be an object")
    kind = value.get("kind")
    if kind == "model_response":
        return litellm.ModelResponse(**cast("dict[str, object]", body))
    if kind == "text_completion":
        return litellm.TextCompletionResponse(
            **cast("dict[str, object]", body)  # pyright: ignore[reportArgumentType]
        )
    if kind == "embedding":
        return litellm.EmbeddingResponse(
            **cast("dict[str, object]", body)  # pyright: ignore[reportArgumentType]
        )
    if kind == "image":
        return litellm.ImageResponse(
            **cast("dict[str, object]", body)  # pyright: ignore[reportArgumentType]
        )
    raise ValueError(f"unknown response kind: {kind}")


def parse_time(value: Json, name: str) -> datetime:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return datetime.fromisoformat(value)


def optional_time(scenario: dict[str, Json], name: str) -> datetime | None:
    value = scenario.get(name)
    return parse_time(value, name) if value is not None else None


def build_kwargs(scenario: dict[str, Json]) -> dict[str, object]:
    metadata = scenario.get("metadata")
    kwargs: dict[str, object] = {
        "litellm_call_id": scenario.get("call_id"),
        "messages": scenario.get("messages"),
        "litellm_params": {"metadata": metadata if isinstance(metadata, dict) else {}},
        "response_cost": scenario.get("response_cost"),
        "call_type": scenario.get("call_type"),
        "api_call_start_time": optional_time(scenario, "api_call_start_time"),
        "completion_start_time": optional_time(scenario, "completion_start_time"),
    }
    standard = scenario.get("standard_logging_object")
    if isinstance(standard, dict):
        kwargs["standard_logging_object"] = standard
    return kwargs


async def log_async(
    logger: BraintrustLogger,
    kwargs: dict[str, object],
    response: ResponseObject | None,
    start: datetime,
    end: datetime,
) -> None:
    log_event = cast(Callable[..., Awaitable[None]], logger.async_log_success_event)  # pyright: ignore[reportUnknownMemberType]
    await log_event(kwargs, response, start, end)


def log_result(scenario: dict[str, Json]) -> Json:
    logger = build_logger(scenario)
    tags = scenario.get("default_tags")
    litellm.langfuse_default_tags = (
        [tag for tag in tags if isinstance(tag, str)] if isinstance(tags, list) else None
    )
    answers = scenario.get("answers")
    recorder = Recorder(list(answers) if isinstance(answers, list) else [])
    kwargs = build_kwargs(scenario)
    response = build_response(scenario.get("response"))
    start = parse_time(scenario.get("start_time"), "start_time")
    end = parse_time(scenario.get("end_time"), "end_time")
    calls = scenario.get("calls")
    count = calls if isinstance(calls, int) and not isinstance(calls, bool) else 1
    sync_call = cast(Callable[..., None], logger.log_success_event)  # pyright: ignore[reportUnknownMemberType]
    with (
        patch.object(logger, "global_braintrust_sync_http_handler", FakeSyncClient(recorder)),
        patch.object(logger, "global_braintrust_http_handler", FakeAsyncClient(recorder)),
    ):
        for _ in range(count):
            if scenario.get("path") == "async":
                asyncio.run(log_async(logger, kwargs, response, start, end))
            else:
                sync_call(kwargs, response, start, end)
    return cast(Json, {"requests": recorder.requests})


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    scenarios = input_value.get("scenarios")
    if not isinstance(scenarios, list):
        raise ValueError("scenarios must be a list")
    handlers = {"resolve_project": project_result, "log": log_result}
    handler = handlers.get(function) if isinstance(function, str) else None
    if handler is None:
        raise ValueError(f"unknown function: {function}")
    return {"results": [handler(cast("dict[str, Json]", value)) for value in scenarios]}
