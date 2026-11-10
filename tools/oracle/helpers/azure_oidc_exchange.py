import json
import os
import time
from typing import NotRequired, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm
from litellm import DualCache
from litellm.llms.azure import common_utils
from litellm.llms.azure.common_utils import AzureOpenAIError, get_azure_ad_token_from_oidc

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

NOW = 1_700_000_000.0
FIXTURE_EXHAUSTED_STATUS = 599


class Reply(TypedDict):
    status: int
    body: str


class Call(TypedDict):
    azure_ad_token: str
    azure_client_id: NotRequired[str | None]
    azure_tenant_id: NotRequired[str | None]
    scope: NotRequired[str | None]
    advance: NotRequired[float]


class Step(TypedDict, total=False):
    environment: dict[str, str]
    calls: list[Call]
    replies: list[Reply]


class Input(TypedDict):
    steps: list[Step]


class RecordedRequest(TypedDict):
    url: str
    form: dict[str, str]


class FixtureClient:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[RecordedRequest] = []

    def post(
        self, url: str, data: dict[str, str] | None = None, **_kwargs: object
    ) -> httpx.Response:
        form = {name: str(value) for name, value in (data or {}).items()}
        self.requests.append({"url": url, "form": form})
        if not self.replies:
            return httpx.Response(FIXTURE_EXHAUSTED_STATUS, content=b"fixture exhausted")
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"].encode())


class FixtureClock:
    def __init__(self) -> None:
        self.now = NOW

    def time(self) -> float:
        return self.now


def error_status(error: Exception) -> int | None:
    carried: object = getattr(error, "status_code", None)
    return carried if isinstance(carried, int) and not isinstance(carried, bool) else None


def call_outcome(call: Call) -> Json:
    try:
        token = get_azure_ad_token_from_oidc(
            call["azure_ad_token"],
            call.get("azure_client_id"),
            call.get("azure_tenant_id"),
            call.get("scope"),
        )
    except (AzureOpenAIError, json.JSONDecodeError) as error:
        return {"kind": "error", "status": error_status(error)}
    return {"kind": "token", "token": cast("Json", token)}


def step_report(step: Step) -> dict[str, Json]:
    client = FixtureClient(list(step.get("replies", [])))
    clock = FixtureClock()
    outcomes: list[Json] = []
    environment = dict(step.get("environment", {}))
    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch.object(time, "time", clock.time),
        mock.patch.object(common_utils, "azure_ad_cache", DualCache()),
        mock.patch.object(litellm, "module_level_client", client),
    ):
        for call in step.get("calls", []):
            clock.now += call.get("advance", 0.0)
            outcomes.append(call_outcome(call))
    return {"outcomes": outcomes, "requests": cast("Json", client.requests)}


def run(argument: Input) -> Json:
    return cast("Json", [step_report(step) for step in argument["steps"]])
