import asyncio
from collections.abc import Mapping, Sequence
from typing import NotRequired, TypedDict

from litellm.proxy.health_check import perform_health_check
from litellm.router import Router
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false
# pyright: reportUnknownVariableType=false


class EndpointRow(TypedDict):
    model_id: str
    error: NotRequired[str]


ENDPOINT = TypeAdapter(EndpointRow)


class Entry(Strict):
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue] = Field(default_factory=dict)


class Row(Strict):
    model_id: str
    error: str | None


class Output(Strict):
    healthy: list[str]
    unhealthy: list[Row]


# perform_health_check's return carries bare list and dict element types, so the typed boundary is this declared shape.
async def checked(
    model_list: list[dict[str, JsonValue]],
    model_id: str | None,
    router: Router,
) -> tuple[Sequence[object], Sequence[object], Mapping[object, object]]:
    return await perform_health_check(model_list=model_list, model_id=model_id, router=router)


def model_row(entry: Entry) -> dict[str, JsonValue]:
    return {
        "model_name": entry.model_name,
        "litellm_params": dict(entry.litellm_params),
        "model_info": dict(entry.model_info),
    }


def reported(row: object) -> Row:
    endpoint = ENDPOINT.validate_python(row)
    return Row(model_id=endpoint["model_id"], error=endpoint.get("error"))


class StrategyRouterHealth(Operation):
    models: list[Entry]
    model_group_alias: dict[str, str] = Field(default_factory=dict)
    model_id: str | None = None
    failed: list[str] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        rows = [model_row(entry) for entry in self.models]
        router = Router(model_list=rows, model_group_alias=dict(self.model_group_alias))
        healthy, unhealthy, _exceptions = asyncio.run(checked(rows, self.model_id, router))
        return Output(
            healthy=[ENDPOINT.validate_python(row)["model_id"] for row in healthy],
            unhealthy=[reported(row) for row in unhealthy],
        )
