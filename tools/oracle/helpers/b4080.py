import asyncio
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.llm_judge import (
    judge_acompletion,
    judge_target,
    parse_json_verdict,
)
from litellm.router import Router
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ParseInput(TypedDict):
    check: Literal["parse"]
    raw: str


class ServedModels(TypedDict):
    model: str
    team_id: NotRequired[str | None]
    models: list[str]


class RouterInput(TypedDict):
    served: list[ServedModels]


class QueryInput(TypedDict):
    model: str
    team_id: NotRequired[str | None]


class DispatchInput(TypedDict):
    model: str
    team_id: NotRequired[str | None]
    messages: list[Json]
    params: NotRequired[dict[str, Json]]


class TargetInput(TypedDict):
    check: Literal["target"]
    router: NotRequired[RouterInput | None]
    queries: list[QueryInput]
    dispatch: NotRequired[DispatchInput | None]


Input: TypeAlias = ParseInput | TargetInput

_JUDGE_REPLY: dict[str, Json] = {
    "id": "judge-reply",
    "choices": [
        {
            "index": 0,
            "message": {
                "role": "assistant",
                "content": '{"preference": "A", "confidence": 0.9}',
            },
        }
    ],
}


class _FakeRouter:
    def __init__(self, served: list[ServedModels]) -> None:
        self.served = served
        self.calls: list[dict[str, Json]] = []

    def resolved_litellm_models(
        self, model_name: str, team_id: str | None = None
    ) -> tuple[str, ...]:
        return tuple(
            served_model
            for entry in self.served
            if entry["model"] == model_name
            and (entry.get("team_id") is None or entry.get("team_id") == team_id)
            for served_model in entry["models"]
        )

    async def acompletion(self, **kwargs: Json) -> Json:
        self.calls.append({"kind": "router", **kwargs})
        return _JUDGE_REPLY


class _SdkRecorder:
    def __init__(self) -> None:
        self.calls: list[dict[str, Json]] = []

    async def __call__(self, **kwargs: Json) -> Json:
        self.calls.append({"kind": "sdk", **kwargs})
        return _JUDGE_REPLY


def _query_value(router: _FakeRouter | None, query: QueryInput) -> Json:
    target = judge_target(
        cast(Router | None, router), query["model"], query.get("team_id")
    )
    return {"via": target.via, "models": list[Json](sorted(target.models))}


def _dispatch_value(router: _FakeRouter | None, dispatch: DispatchInput) -> Json:
    sdk = _SdkRecorder()
    with patch.object(litellm, "acompletion", sdk):
        asyncio.run(
            judge_acompletion(
                cast(Router | None, router),
                dispatch["model"],
                cast(list[AllMessageValues], dispatch["messages"]),
                dispatch.get("team_id"),
                **dispatch.get("params", {}),
            )
        )
    if sdk.calls:
        return sdk.calls[0]
    if router is not None and router.calls:
        return router.calls[0]
    return {"declined": True}


def run(input_value: Input) -> Json:
    if input_value["check"] == "parse":
        return cast(Json, parse_json_verdict(input_value["raw"]))
    # an unmapped model name makes get_llm_provider print to stdout, where the answer goes
    litellm.suppress_debug_info = True
    router_input = input_value.get("router")
    router = _FakeRouter(router_input["served"]) if router_input is not None else None
    queries: list[Json] = [
        _query_value(router, query) for query in input_value["queries"]
    ]
    result: dict[str, Json] = {"queries": queries}
    dispatch = input_value.get("dispatch")
    if dispatch is not None:
        result["dispatch"] = _dispatch_value(router, dispatch)
    return result
