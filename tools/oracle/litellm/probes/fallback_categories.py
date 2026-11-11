from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING, Annotated, Final, Literal, cast
from unittest.mock import patch

import litellm.router
from litellm.exceptions import (
    ContentPolicyViolationError,
    ContextWindowExceededError,
    GuardrailRaisedException,
    RateLimitError,
)
from litellm.router import Router
from litellm.router_utils.fallback_event_handlers import (
    AttemptedFallbackTargets,
    fallback_lookup_groups,
)
from litellm.types.utils import Choices, Message, ModelResponse
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

if TYPE_CHECKING:
    from collections.abc import Callable

    from litellm.types.litellm_params import RoutingStrategyName
    from litellm.types.router import DeploymentTypedDict, LiteLLMParamsTypedDict
    from typing_extensions import Protocol

    class FailedDeployment(Protocol):
        failed_deployment_id: str | None

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MESSAGE: Final = "probe failure"
_MODEL: Final = "openai/gpt-4o"
_PROVIDER: Final = "openai"
_ANTHROPIC_FUNCTION: Final = "anthropic_messages"
_UNMATCHED_GROUP: Final = "__betterlite_unmatched_group__"
_PROBE_API_KEY: Final = "sk-probe"
# pydantic warns when it first builds ModelResponse, and protocol.answer captures warnings only around run()
_WARMED_RESPONSE: Final = ModelResponse(model=_MODEL)


class Target(Strict):
    model: str
    target_order: int | None = None
    excluded_deployment_ids: list[str] = Field(default_factory=list)


class Chain(Strict):
    state: Literal["absent", "unresolved", "resolved"]
    targets: list[Target] = Field(default_factory=list)
    has_unattempted: bool = False


class OrdinaryAbsent(Strict):
    kind: Literal["absent"] = "absent"


class OrdinaryStandard(Strict):
    kind: Literal["standard"] = "standard"
    chain: Chain


class OrdinaryNonStandard(Strict):
    kind: Literal["non_standard"] = "non_standard"
    chain: Chain
    client_targets: list[JsonValue] = Field(default_factory=list)


Ordinary = Annotated[
    OrdinaryAbsent | OrdinaryStandard | OrdinaryNonStandard, Field(discriminator="kind")
]


class Order(Strict):
    model_group: str
    deployment_orders: list[int | None] = Field(default_factory=list)
    current_target: int | None = None
    external: Literal["absent", "targets", "client_fallbacks"]
    external_targets: list[Target] = Field(default_factory=list)


class Weighted(Strict):
    enabled: bool = False
    strategy: str
    failed_deployment_id: str | None = None
    model_group: str
    deployment_ids: list[str] = Field(default_factory=list)
    cooldown_ids: list[str] = Field(default_factory=list)
    metadata_key: Literal["metadata", "litellm_metadata"]
    prior_excluded_ids: list[str] = Field(default_factory=list)


class Response(Strict):
    has_choices: bool = False
    first_finish_reason: str | None = None
    function_name: str
    refusal: Literal["not_refusal", "without_stop_details", "with_stop_details"]


class SuccessorStop(Strict):
    outcome: Literal["stop"] = "stop"


class SuccessorRun(Strict):
    outcome: Literal["run"] = "run"
    targets: list[Target] = Field(default_factory=list)
    client_fallbacks_after: bool = False


Successor = Annotated[SuccessorStop | SuccessorRun, Field(discriminator="outcome")]


class FallbackAvailability(Strict):
    default_fallbacks: bool
    content_policy: bool


class RefusalAvailability(Strict):
    available: bool


class RaiseDecision(Strict):
    content_policy: bool
    anthropic_refusal: bool


class MetadataOutput(Strict):
    bucket: str
    excluded_deployment_ids: list[str]


class Output(Strict):
    successor: Successor
    fallback: FallbackAvailability
    refusal: RefusalAvailability
    raise_decision: RaiseDecision
    metadata: MetadataOutput


def anthropic_messages() -> None:
    pass


def other_function() -> None:
    pass


class FallbackCategories(Operation):
    failure: Literal["context_window", "content_policy", "ordinary"]
    disable_fallbacks: bool = False
    guardrail_intervention: bool = False
    original_model_group: str | None
    model_group: str
    pre_routing_selected: str | None = None
    router_fallbacks: list[str] = Field(default_factory=list)
    ordinary: Ordinary
    context_window: Chain
    content_policy: Chain
    order: Order
    weighted_failover: Weighted
    attempted: list[str] = Field(default_factory=list)
    response: Response

    @override
    def run(self) -> Output:
        router, kwargs = self.scenario()
        successor = self.successor(router, kwargs)
        return Output(
            successor=successor,
            metadata=self.metadata_output(kwargs),
            fallback=FallbackAvailability(
                default_fallbacks=router._has_default_fallbacks(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                content_policy=router._has_content_policy_fallback(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    self.model_group, kwargs
                ),
            ),
            refusal=RefusalAvailability(
                available=router._refusal_fallback_available(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    self.model_group, kwargs
                )
            ),
            raise_decision=RaiseDecision(
                content_policy=router._should_raise_content_policy_error(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                    self.model_group, self.model_response(), kwargs
                ),
                anthropic_refusal=router._should_raise_anthropic_refusal_error(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
                    self.model_group, self.target_function(), self.refusal_payload(), kwargs
                ),
            ),
        )

    def scenario(self) -> tuple[Router, dict[str, object]]:
        bucket = self.metadata()
        kwargs: dict[str, object] = {
            "model": self.original_model_group,
            "disable_fallbacks": self.disable_fallbacks,
            "fallbacks": self.ordinary_param(),
            "context_window_fallbacks": self.chain_param(self.context_window),
            "content_policy_fallbacks": self.chain_param(self.content_policy),
            "attempted_targets": AttemptedFallbackTargets(frozenset(self.attempted)),
        }
        # the resume hop carries its target order in this router key
        if self.order.current_target is not None:
            kwargs["_target_order"] = self.order.current_target
        if bucket:
            kwargs[self.bucket_name()] = bucket
        router = Router(
            model_list=self.model_entries(),
            routing_strategy=cast("RoutingStrategyName", self.weighted_failover.strategy),
            enable_weighted_failover=self.weighted_failover.enabled,
            fallbacks=[{group: ["probe-default"]} for group in self.router_fallbacks],
        )
        return router, kwargs

    def metadata(self) -> dict[str, object]:
        bucket: dict[str, object] = {}
        if self.pre_routing_selected is not None:
            bucket["pre_routing_selected_model"] = self.pre_routing_selected
        if self.weighted_failover.prior_excluded_ids:
            bucket["_failover_excluded_ids"] = list(self.weighted_failover.prior_excluded_ids)
        return bucket

    def bucket_name(self) -> str:
        return self.weighted_failover.metadata_key

    def lookup_groups(self) -> tuple[str, ...]:
        preview: dict[str, object] = {"model": self.original_model_group}
        bucket = self.metadata()
        if bucket:
            preview[self.bucket_name()] = bucket
        return fallback_lookup_groups(preview, self.model_group)

    def model_entries(self) -> list[DeploymentTypedDict]:
        if self.weighted_failover.cooldown_ids:
            raise ProbeError(
                "invalid_request",
                "the fallback_categories probe cannot seed cooled-down deployments",
            )
        orders = self.order.deployment_orders
        ids = self.weighted_failover.deployment_ids
        if len(orders) != len(ids):
            raise ProbeError(
                "invalid_request",
                "deployment_orders and deployment_ids describe the same deployments",
            )
        entries: list[DeploymentTypedDict] = []
        for order, deployment_id in zip(orders, ids, strict=True):
            params: LiteLLMParamsTypedDict = {"model": _MODEL, "api_key": _PROBE_API_KEY}
            if order is not None:
                params["order"] = order
            entries.append(
                {
                    "model_name": self.order.model_group,
                    "litellm_params": params,
                    "model_info": {"id": deployment_id},
                }
            )
        return entries

    def chain_param(self, chain: Chain) -> list[JsonValue] | None:
        if chain.state == "absent":
            return None
        if chain.state == "unresolved":
            return [{_UNMATCHED_GROUP: ["unused"]}]
        groups = self.lookup_groups()
        if not groups:
            raise ProbeError(
                "invalid_request", "a resolved chain needs a lookup group"
            )
        return [{groups[0]: [self.wire_target(target) for target in chain.targets]}]

    def ordinary_param(self) -> list[JsonValue] | None:
        if isinstance(self.ordinary, OrdinaryAbsent):
            return None
        if isinstance(self.ordinary, OrdinaryNonStandard):
            return self.ordinary.client_targets
        return self.chain_param(self.ordinary.chain)

    def wire_target(self, target: Target) -> dict[str, JsonValue]:
        entry: dict[str, JsonValue] = {"model": target.model}
        if target.target_order is not None:
            entry["_target_order"] = target.target_order
        if target.excluded_deployment_ids:
            entry["_excluded_deployment_ids"] = list(target.excluded_deployment_ids)
        return entry

    def build_exception(self) -> Exception:
        if self.guardrail_intervention:
            exception: Exception = GuardrailRaisedException(
                guardrail_name="probe", message=_MESSAGE
            )
        elif self.failure == "context_window":
            exception = ContextWindowExceededError(
                message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
            )
        elif self.failure == "content_policy":
            exception = ContentPolicyViolationError(
                message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
            )
        else:
            exception = RateLimitError(
                message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER
            )
        # B017 stamps the failed deployment on the error before the dispatcher sees it
        failed = self.weighted_failover.failed_deployment_id
        if failed is not None:
            cast("FailedDeployment", cast("object", exception)).failed_deployment_id = (
                failed
            )
        return exception

    def successor(self, router: Router, kwargs: dict[str, object]) -> Successor:
        exception = self.build_exception()
        recorded: list[object] = []

        async def fake_run(*_args: object, **call: object) -> object:
            recorded.append(call.get("fallback_model_group"))
            return {"probe": "response"}

        async def dispatch() -> None:
            with patch.object(litellm.router, "run_async_fallback", fake_run):
                try:
                    await router.async_function_with_fallbacks_common_utils(  # pyright: ignore[reportUnknownMemberType]
                        exception,
                        self.disable_fallbacks,
                        self.ordinary_param(),
                        self.chain_param(self.context_window),
                        self.chain_param(self.content_policy),
                        self.model_group,
                        (),
                        kwargs,
                    )
                except Exception as raised:
                    if raised is not exception:
                        raise ProbeError(
                            "invalid_request",
                            f"the fallback dispatcher raised {type(raised).__name__}: {raised}",
                        ) from raised
                    return
                if not recorded:
                    raise ProbeError(
                        "invalid_request",
                        "the fallback dispatcher returned without running a fallback",
                    )

        asyncio.run(dispatch())
        if not recorded:
            return SuccessorStop()
        return self.recorded_successor(recorded[0])

    def metadata_output(self, kwargs: dict[str, object]) -> MetadataOutput:
        bucket = kwargs.get(self.bucket_name())
        excluded: list[str] = []
        if isinstance(bucket, dict):
            stored = cast("dict[str, object]", bucket).get("_failover_excluded_ids")
            if isinstance(stored, list):
                excluded = [
                    item
                    for item in cast("list[object]", stored)
                    if isinstance(item, str)
                ]
        return MetadataOutput(
            bucket=self.bucket_name(),
            excluded_deployment_ids=sorted(excluded),
        )

    def recorded_successor(self, entries: object) -> Successor:
        if not isinstance(entries, list):
            raise ProbeError("invalid_request", "the fallback dispatcher ran a non-list chain")
        chain = cast("list[JsonValue]", entries)
        client = self.client_chain()
        if client is not None and chain is client:
            return SuccessorRun(
                targets=[], client_fallbacks_after=True
            )
        tail = 0
        if client is not None and len(client) <= len(chain):
            size = len(client)
            if size > 0 and all(
                chain[-size + index] is entry
                for index, entry in enumerate(client)
            ):
                tail = size
        targets = [
            self.recorded_target(entry)
            for index, entry in enumerate(chain)
            if index < len(chain) - tail
        ]
        return SuccessorRun(targets=targets, client_fallbacks_after=tail > 0)

    def client_chain(self) -> list[JsonValue] | None:
        if isinstance(self.ordinary, OrdinaryNonStandard):
            return self.ordinary.client_targets
        return None

    def recorded_target(self, entry: JsonValue) -> Target:
        if isinstance(entry, str):
            return Target(model=entry)
        if isinstance(entry, dict):
            model = entry.get("model")
            if not isinstance(model, str):
                raise ProbeError("invalid_request", "a fallback entry names no model")
            order = entry.get("_target_order")
            excluded = entry.get("_excluded_deployment_ids")
            extra = set(entry) - {"model", "_target_order", "_excluded_deployment_ids"}
            if extra:
                raise ProbeError(
                    "invalid_request", "the probe cannot compare parameterized entries"
                )
            # LiteLLM builds this list from a set, so membership is the contract, not order
            excluded_ids = (
                [item for item in excluded if isinstance(item, str)]
                if isinstance(excluded, list)
                else []
            )
            return Target(
                model=model,
                target_order=order if isinstance(order, int) else None,
                excluded_deployment_ids=sorted(excluded_ids),
            )
        raise ProbeError("invalid_request", "a fallback entry has an unknown shape")

    def model_response(self) -> ModelResponse:
        choices = (
            [
                Choices(
                    finish_reason=self.response.first_finish_reason,
                    index=0,
                    message=Message(content="", role="assistant"),
                )
            ]
            if self.response.has_choices
            else []
        )
        return ModelResponse(model=_MODEL, choices=choices)

    def target_function(self) -> Callable[[], None]:
        if self.response.function_name == _ANTHROPIC_FUNCTION:
            return anthropic_messages
        return other_function

    def refusal_payload(self) -> dict[str, object]:
        if self.response.refusal == "not_refusal":
            return {"stop_reason": "end_turn"}
        if self.response.refusal == "without_stop_details":
            return {"stop_reason": "refusal"}
        return {"stop_reason": "refusal", "stop_details": {"type": "refusal"}}
