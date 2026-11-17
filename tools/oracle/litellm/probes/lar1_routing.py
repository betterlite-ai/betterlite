import asyncio
from typing import ClassVar, Protocol

from litellm.router import Router
from pydantic import ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

API_KEY = "sk-probe"
GROUP = "probe-group"
MAX_PADDING_BYTES = 1 << 22
Args = TypeAdapter(dict[str, JsonValue])
Value = TypeAdapter(JsonValue)


class Deployment(Strict):
    id: str
    model: str
    kind: str | None = None


class ReturnedInfo(Strict):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    id: str | None = None


class Returned(Strict):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")
    model_info: ReturnedInfo = ReturnedInfo()


class Selected(Strict):
    selected: str | None = None
    error: str | None = None


class RouterView(Protocol):
    async def async_get_available_deployment(
        self,
        model: str,
        request_kwargs: dict[str, object],
        specific_deployment: bool,
    ) -> object: ...


def model_list_entry(group: str, deployment: Deployment) -> dict[str, object]:
    info: dict[str, object] = {"id": deployment.id}
    if deployment.kind is not None:
        info["type"] = deployment.kind
    params: dict[str, object] = {"model": deployment.model, "api_key": API_KEY}
    return {"model_name": group, "litellm_params": params, "model_info": info}


class Lar1Routing(Operation):
    model: str
    group: str = GROUP
    specific_deployment: bool = False
    deployments: list[Deployment]
    routing_strategy_args: str | None = None
    metadata: str | None = None
    padding_bytes: int = Field(default=0, ge=0, le=MAX_PADDING_BYTES)

    @override
    def run(self) -> Selected:
        return asyncio.run(self.execute())

    async def execute(self) -> Selected:
        args = {} if self.routing_strategy_args is None else Args.validate_json(self.routing_strategy_args)
        entries = [model_list_entry(self.group, deployment) for deployment in self.deployments]
        try:
            router = Router(model_list=entries, routing_strategy="lar1", routing_strategy_args=args)
        except ValueError as error:
            return Selected(error=str(error))
        return Selected(selected=await self.answer(router))

    async def answer(self, router: RouterView) -> str | None:
        request_kwargs: dict[str, object] = {}
        if self.metadata is not None:
            lar1 = Value.validate_json(self.metadata + " " * self.padding_bytes)
            request_kwargs["metadata"] = {"lar1": lar1}
        deployment = await router.async_get_available_deployment(
            model=self.model,
            request_kwargs=request_kwargs,
            specific_deployment=self.specific_deployment,
        )
        if deployment is None:
            return None
        returned = TypeAdapter(Returned).validate_python(deployment)
        return returned.model_info.id
