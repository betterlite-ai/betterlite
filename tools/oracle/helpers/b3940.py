import asyncio
import os
from collections.abc import Sequence
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.integrations.datadog.datadog_cost_management import (
    DatadogCostManagementLogger,
)
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

BODY_ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

CONTENT_TYPE_HEADER = "content-type"
API_KEY_HEADER = "dd-api-key"
APP_KEY_HEADER = "dd-application-key"
CLEARED_ENVIRONMENT = ("HOSTNAME", "POD_NAME")
CLEARED_PREFIXES = ("DD_", "LITELLM_DD_")


class Reply(TypedDict):
    status: int
    body: str


class Init(TypedDict, total=False):
    cost_tag_keys: list[str]
    batch_size: int
    flush_interval: float
    max_queue_size: int


class SuccessAction(TypedDict):
    do: Literal["success"]
    payload: Json


class SendBatchAction(TypedDict):
    do: Literal["send_batch"]


class FlushAction(TypedDict):
    do: Literal["flush"]


Action: TypeAlias = SuccessAction | SendBatchAction | FlushAction


class Scenario(TypedDict):
    name: str
    environment: dict[str, str]
    init: Init
    replies: list[Reply]
    actions: list[Action]


class Input(TypedDict):
    scenarios: list[Scenario]


class Call(TypedDict):
    url: str
    method: str
    content_type: str | None
    api_key: str | None
    app_key: str | None
    body: JsonValue


class InitOutput(TypedDict):
    upload_url: str
    site: str
    cost_tag_keys: list[str]
    batch_size: int
    flush_interval: float
    max_queue_size: int
    authenticated: bool


class Result(TypedDict):
    queue_length: int


class ScenarioOutput(TypedDict):
    name: str
    init: InitOutput
    results: list[Result]
    calls: list[Call]
    queue_length: int


class Output(TypedDict):
    scenarios: list[ScenarioOutput]


class LoggerProtocol(Protocol):
    @property
    def log_queue(self) -> Sequence[Json]: ...

    async def async_log_success_event(
        self,
        kwargs: dict[str, Json],
        response_obj: None,
        start_time: None,
        end_time: None,
    ) -> None: ...

    async def async_send_batch(self) -> None: ...

    async def flush_queue(self) -> None: ...


class Script:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = replies
        self.calls: list[Call] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        body = BODY_ADAPTER.validate_json(request.content)
        self.calls.append(
            {
                "url": str(request.url),
                "method": request.method,
                "content_type": request.headers.get(CONTENT_TYPE_HEADER),
                "api_key": request.headers.get(API_KEY_HEADER),
                "app_key": request.headers.get(APP_KEY_HEADER),
                "body": body,
            }
        )
        if not self.replies:
            raise ValueError("the scenario has no scripted reply left for this upload")
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], text=reply["body"], request=request)


async def perform(logger: LoggerProtocol, action: Action) -> Result:
    if action["do"] == "success":
        await logger.async_log_success_event(
            {"standard_logging_object": action["payload"]},
            None,
            None,
            None,
        )
    elif action["do"] == "send_batch":
        await logger.async_send_batch()
    else:
        await logger.flush_queue()
    return {"queue_length": len(logger.log_queue)}


async def execute(scenario: Scenario, script: Script) -> ScenarioOutput:
    logger = DatadogCostManagementLogger(**scenario["init"])
    logger.async_client = AsyncHTTPHandler(transport=httpx.MockTransport(script))
    calls: LoggerProtocol = logger
    results = [await perform(calls, action) for action in scenario["actions"]]
    return {
        "name": scenario["name"],
        "init": {
            "upload_url": logger.upload_url,
            "site": logger.dd_site,
            "cost_tag_keys": list(logger.cost_tag_keys),
            "batch_size": int(logger.batch_size),
            "flush_interval": float(logger.flush_interval),
            "max_queue_size": int(logger.max_queue_size),
            "authenticated": bool(logger.dd_api_key and logger.dd_app_key),
        },
        "results": results,
        "calls": script.calls,
        "queue_length": results[-1]["queue_length"] if results else 0,
    }


def run_scenario(scenario: Scenario) -> ScenarioOutput:
    script = Script(scenario["replies"])
    for name in list(os.environ):
        if name.startswith(CLEARED_PREFIXES) or name in CLEARED_ENVIRONMENT:
            del os.environ[name]
    os.environ.update(scenario["environment"])
    return asyncio.run(execute(scenario, script))


def run(input_value: Input) -> Output:
    return {
        "scenarios": [
            run_scenario(scenario) for scenario in input_value["scenarios"]
        ]
    }
