import asyncio
from collections.abc import Callable, Coroutine
from typing import Any, TypeAlias, cast

from litellm.integrations.websearch_interception.handler import WebSearchInterceptionLogger

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

AgenticLoopGate: TypeAlias = Callable[
    ..., Coroutine[Any, Any, tuple[bool, dict[str, Json]]]
]


def run(input_value: dict[str, Json]) -> Json:
    enabled_providers = cast(list[str], input_value["enabled_providers"])
    logger = WebSearchInterceptionLogger(enabled_providers=enabled_providers)
    kwargs: dict[str, Json] = {}
    surface = input_value.get("surface")
    if surface is not None:
        kwargs["_agentic_loop_api_surface"] = surface
    gate = cast(AgenticLoopGate, logger.async_should_run_agentic_loop)
    should_run, tools_dict = asyncio.run(
        gate(
            response=input_value["response"],
            model="test-model",
            messages=[],
            tools=cast("list[dict[str, Json]] | None", input_value.get("tools")),
            stream=bool(input_value.get("stream", False)),
            custom_llm_provider=cast(str, input_value["custom_llm_provider"]),
            kwargs=kwargs,
        )
    )
    output: dict[str, Json] = {"should_run": should_run}
    if should_run:
        output["tools_dict"] = cast(Json, tools_dict)
    return output
