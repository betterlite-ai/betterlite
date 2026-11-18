import asyncio
import logging
import os
import sys
from collections.abc import Generator, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

from litellm.proxy._types import ProxyException, ReconcileOutcome
from litellm.proxy.management_endpoints.model_management_endpoints import (
    clear_cache,
    model_info_as_mapping,
    raise_if_reload_degraded_serving,
    reload_serving_verdict,
)
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
NO_OUTCOME: Final = ReconcileOutcome(still_desired=None, live_after=None)
HELPER_LOGGER: Final = logging.Logger("betterlite.oracle.model_reload")
HELPER_LOGGER.addHandler(logging.NullHandler())
HELPER_LOGGER.propagate = False


class Tag(TypedDict):
    function: str


class ModelRow(TypedDict):
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


class LiveInput(TypedDict):
    captured: NotRequired[list[str] | None]
    router_ids: NotRequired[list[str] | None]


class WrittenInput(TypedDict):
    id: str
    model_info: JsonValue


class OutcomeInput(TypedDict):
    still_desired: NotRequired[list[str] | None]
    live_after: NotRequired[list[str] | None]


class MappingCall(TypedDict):
    function: Literal["model_info_as_mapping"]
    model_info: JsonValue


class VerdictCall(TypedDict):
    function: Literal["reload_serving_verdict"]
    obligation: Literal["must_serve", "must_not_degrade"]
    before: list[str]
    written: list[WrittenInput]
    still_desired: NotRequired[list[str] | None]
    live: LiveInput


class RaiseCall(TypedDict):
    function: Literal["raise_if_reload_degraded_serving"]
    action: str
    before: list[str]
    written: list[WrittenInput]
    still_desired: NotRequired[list[str] | None]
    live: LiveInput


class ClearCacheCall(TypedDict):
    function: Literal["clear_cache"]
    router_present: bool
    database_present: bool
    models: list[ModelRow]
    strategy_names: list[str]
    outcome: OutcomeInput
    reload_error: NotRequired[str | None]


class Input(TypedDict):
    environment: dict[str, str]
    calls: list[JsonValue]


TAG: Final = TypeAdapter(Tag)
MAPPING_CALL: Final = TypeAdapter(MappingCall)
VERDICT_CALL: Final = TypeAdapter(VerdictCall)
RAISE_CALL: Final = TypeAdapter(RaiseCall)
CLEAR_CACHE_CALL: Final = TypeAdapter(ClearCacheCall)
JSON: Final = TypeAdapter(JsonValue)
ID: Final = TypeAdapter(str)


class FakeDatabase:
    pass


@dataclass
class ReloadConfig:
    outcome: ReconcileOutcome
    error: str | None
    reloads: int = 0

    async def _add_deployment_locked(
        self, *, prisma_client: FakeDatabase, proxy_logging_obj: logging.Logger
    ) -> ReconcileOutcome:
        del prisma_client, proxy_logging_obj
        self.reloads += 1
        if self.error is not None:
            raise RuntimeError(self.error)
        return self.outcome


class FakeRouter:
    model_list: list[ModelRow]
    deleted: list[str]
    auto_routers: dict[str, logging.Logger]
    complexity_routers: dict[str, logging.Logger]
    adaptive_routers: dict[str, logging.Logger]
    quality_routers: dict[str, logging.Logger]

    def __init__(self, models: Sequence[ModelRow], strategy_names: Sequence[str]) -> None:
        self.model_list = [
            ModelRow(
                model_name=model["model_name"],
                litellm_params=dict(model["litellm_params"]),
                model_info=dict(model["model_info"]),
            )
            for model in models
        ]
        self.deleted = []
        self.auto_routers = dict.fromkeys(strategy_names, HELPER_LOGGER)
        self.complexity_routers = dict.fromkeys(strategy_names, HELPER_LOGGER)
        self.adaptive_routers = dict.fromkeys(strategy_names, HELPER_LOGGER)
        self.quality_routers = dict.fromkeys(strategy_names, HELPER_LOGGER)

    def get_model_ids(self) -> list[str]:
        return [
            ID.validate_python(model["model_info"]["id"])
            for model in self.model_list
            if "id" in model["model_info"]
        ]

    def delete_deployment(self, **fields: str) -> None:
        model_id = fields["id"]
        self.deleted.append(model_id)
        self.model_list = [
            model for model in self.model_list if model["model_info"].get("id") != model_id
        ]

    def registries(self) -> dict[str, list[str]]:
        return {
            "adaptive": sorted(self.adaptive_routers),
            "auto": sorted(self.auto_routers),
            "complexity": sorted(self.complexity_routers),
            "quality": sorted(self.quality_routers),
        }


class ProxyServer(ModuleType):
    llm_router: FakeRouter | None = None
    prisma_client: FakeDatabase | None = None
    proxy_config: ReloadConfig | None = None
    proxy_logging_obj: logging.Logger | None = None
    verbose_proxy_logger: logging.Logger | None = None
    MODEL_RECONCILE_LOCK: asyncio.Lock | None = None


@contextmanager
def fake_proxy(
    router: FakeRouter | None,
    database: FakeDatabase | None,
    config: ReloadConfig,
) -> Generator[None]:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.llm_router = router
    proxy_server.prisma_client = database
    proxy_server.proxy_config = config
    proxy_server.proxy_logging_obj = HELPER_LOGGER
    proxy_server.verbose_proxy_logger = HELPER_LOGGER
    proxy_server.MODEL_RECONCILE_LOCK = asyncio.Lock()
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    try:
        yield
    finally:
        if saved is None:
            del sys.modules[PROXY_MODULE]
        else:
            sys.modules[PROXY_MODULE] = saved


def frozen(ids: list[str] | None) -> frozenset[str] | None:
    if ids is None:
        return None
    return frozenset(ids)


def live_router(live: LiveInput) -> FakeRouter | None:
    router_ids = live.get("router_ids")
    if router_ids is None:
        return None
    models = [
        ModelRow(model_name=model_id, litellm_params={}, model_info={"id": model_id})
        for model_id in router_ids
    ]
    return FakeRouter(models=models, strategy_names=[])


def verdict_answer(call: VerdictCall) -> JsonValue:
    config = ReloadConfig(outcome=NO_OUTCOME, error=None)
    with fake_proxy(live_router(call["live"]), None, config):
        missing, collateral = reload_serving_verdict(
            before=frozenset(call["before"]),
            written_models=[(model["id"], model["model_info"]) for model in call["written"]],
            written_must_serve=call["obligation"] == "must_serve",
            still_desired=frozen(call.get("still_desired")),
            live_after=frozen(call["live"].get("captured")),
        )
    return JSON.validate_python({"missing": list(missing), "collateral": list(collateral)})


def raise_answer(call: RaiseCall) -> JsonValue:
    config = ReloadConfig(outcome=NO_OUTCOME, error=None)
    with fake_proxy(live_router(call["live"]), None, config):
        try:
            raise_if_reload_degraded_serving(
                before=frozenset(call["before"]),
                written_models=[(model["id"], model["model_info"]) for model in call["written"]],
                action=call["action"],
                still_desired=frozen(call.get("still_desired")),
                live_after=frozen(call["live"].get("captured")),
            )
        except ProxyException as error:
            return {"raised": True, "type": error.type, "param": error.param, "code": error.code}
    return {"raised": False, "type": None, "param": None, "code": None}


def clear_cache_answer(call: ClearCacheCall) -> JsonValue:
    config = ReloadConfig(
        outcome=ReconcileOutcome(
            still_desired=frozen(call["outcome"].get("still_desired")),
            live_after=frozen(call["outcome"].get("live_after")),
        ),
        error=call.get("reload_error"),
    )
    router = FakeRouter(call["models"], call["strategy_names"])
    database = FakeDatabase() if call["database_present"] else None
    with fake_proxy(router if call["router_present"] else None, database, config):
        returned = asyncio.run(clear_cache())
    deleted: list[str] = router.deleted if call["router_present"] else []
    remaining: list[str] = router.get_model_ids() if call["router_present"] else []
    registries: dict[str, list[str]] = (
        router.registries()
        if call["router_present"]
        else {"adaptive": [], "auto": [], "complexity": [], "quality": []}
    )
    desired = None if returned.still_desired is None else sorted(returned.still_desired)
    live = None if returned.live_after is None else sorted(returned.live_after)
    return JSON.validate_python(
        {
            "deleted": deleted,
            "remaining": remaining,
            "registries": registries,
            "reloads": config.reloads,
            "outcome": {"still_desired": desired, "live_after": live},
        }
    )


def answer(call: JsonValue) -> JsonValue:
    function = TAG.validate_python(call)["function"]
    if function == "model_info_as_mapping":
        mapping = model_info_as_mapping(MAPPING_CALL.validate_python(call)["model_info"])
        return JSON.validate_python(mapping)
    if function == "reload_serving_verdict":
        return verdict_answer(VERDICT_CALL.validate_python(call))
    if function == "raise_if_reload_degraded_serving":
        return raise_answer(RAISE_CALL.validate_python(call))
    if function == "clear_cache":
        return clear_cache_answer(CLEAR_CACHE_CALL.validate_python(call))
    raise ValueError(f"unknown function {function}")


def run(input_value: Input) -> JsonValue:
    declared = input_value["environment"].get("LITELLM_ENVIRONMENT")
    if os.environ.get("LITELLM_ENVIRONMENT") != declared:
        raise ValueError("the case env and the helper input disagree on LITELLM_ENVIRONMENT")
    answers: list[JsonValue] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
