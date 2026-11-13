from __future__ import annotations

from typing import Final, cast

from litellm.router import Router
from litellm.router_utils.fallback_event_handlers import (
    AttemptedFallbackTargets,
    carry_over_pre_routing_selection,
    clear_pre_routing_selection,
    fallback_attempt_key,
    fallback_lookup_groups,
    fallbacks_disabled_for_request,
    get_fallback_model_group_for_lookup_groups,
    get_pre_routing_selection,
    has_unattempted_fallback_target,
    mid_stream_fallback_hop_kwargs,
    per_request_fallback_controls,
    record_disable_fallbacks,
    record_pre_routing_selection,
)
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_PRE_ROUTING_KEY: Final = "pre_routing_selected_model"
_DISABLE_FALLBACKS_KEY: Final = "_disable_fallbacks"


def probe_function() -> None:
    pass


class Bucket(Strict):
    present: bool = False
    pre_routing_selected_model: str | None = None
    model_group: str | None = None
    original_model_group: str | None = None
    disable_fallbacks: bool = False

    def mapping(self) -> dict[str, object]:
        values: dict[str, object] = {}
        if self.pre_routing_selected_model is not None:
            values[_PRE_ROUTING_KEY] = self.pre_routing_selected_model
        if self.model_group is not None:
            values["model_group"] = self.model_group
        if self.original_model_group is not None:
            values["original_model_group"] = self.original_model_group
        if self.disable_fallbacks:
            values[_DISABLE_FALLBACKS_KEY] = True
        return values


class State(Strict):
    model: str | None = None
    lookup_model_group: str | None = None
    metadata: Bucket = Field(default_factory=Bucket)
    litellm_metadata: Bucket = Field(default_factory=Bucket)
    disable_fallbacks: bool = False

    def kwargs(self) -> dict[str, object]:
        kwargs: dict[str, object] = {}
        if self.model is not None:
            kwargs["model"] = self.model
        if self.litellm_metadata.present:
            kwargs["litellm_metadata"] = self.litellm_metadata.mapping()
        if self.metadata.present:
            kwargs["metadata"] = self.metadata.mapping()
        kwargs["disable_fallbacks"] = self.disable_fallbacks
        return kwargs


class PlanConfig(Strict):
    fallbacks: list[JsonValue] = Field(default_factory=list)
    context_window_fallbacks: list[JsonValue] = Field(default_factory=list)
    content_policy_fallbacks: list[JsonValue] = Field(default_factory=list)
    max_fallbacks: int
    depth: int = 0


class RequestChains(Strict):
    fallbacks: list[JsonValue] | None = None
    context_window_fallbacks: list[JsonValue] | None = None
    content_policy_fallbacks: list[JsonValue] | None = None
    num_retries: int | None = None
    model_group_retry_policy: JsonValue | None = None


class ChainOutput(Strict):
    resolved: bool
    targets: list[JsonValue]
    has_unattempted: bool


class BudgetOutput(Strict):
    depth: int
    max_fallbacks: int
    exhausted: bool


class BucketOutput(Strict):
    pre_routing_selected_model: str | None
    disable_fallbacks: bool


class HopOutput(Strict):
    model: str
    fallbacks: list[JsonValue] | None
    context_window_fallbacks: list[JsonValue] | None
    content_policy_fallbacks: list[JsonValue] | None
    num_retries: int | None
    model_group_retry_policy: JsonValue | None


class Output(Strict):
    lookup_groups: list[str]
    ordinary: ChainOutput
    context_window: ChainOutput
    content_policy: ChainOutput
    disabled: bool
    budget: BudgetOutput
    metadata: BucketOutput
    litellm_metadata: BucketOutput
    carried: str | None
    hop: HopOutput


def attempt_targets(entries: list[JsonValue]) -> AttemptedFallbackTargets:
    keys: set[str] = set()
    for entry in entries:
        key = fallback_attempt_key(entry)
        if key is not None:
            keys.add(key)
    return AttemptedFallbackTargets(frozenset(keys))


def chain_of(value: object) -> list[JsonValue] | None:
    if value is None:
        return None
    if not isinstance(value, list):
        raise ProbeError("invalid_request", "a fallback chain is not a list")
    return cast("list[JsonValue]", value)


def bucket_output(bucket: object) -> BucketOutput:
    if not isinstance(bucket, dict):
        return BucketOutput(pre_routing_selected_model=None, disable_fallbacks=False)
    values = cast("dict[str, object]", bucket)
    selected = values.get(_PRE_ROUTING_KEY)
    return BucketOutput(
        pre_routing_selected_model=selected if isinstance(selected, str) else None,
        disable_fallbacks=values.get(_DISABLE_FALLBACKS_KEY) is True,
    )


def hop_field(values: dict[str, object], key: str) -> list[JsonValue] | None:
    value = values.get(key)
    if value is None:
        return None
    if not isinstance(value, list):
        raise ProbeError("invalid_request", f"the hop field {key} is not a chain")
    return cast("list[JsonValue]", value)


class FallbackPlans(Operation):
    state: State
    snapshot: State = Field(default_factory=State)
    config: PlanConfig
    request: RequestChains
    attempted: list[JsonValue] = Field(default_factory=list)
    record_selection: str | None = None
    record_disable: bool | None = None

    @override
    def run(self) -> Output:
        kwargs = self.scenario()
        groups = fallback_lookup_groups(kwargs, self.state.lookup_model_group)
        ordinary = self.resolve(chain_of(kwargs.get("fallbacks", self.config.fallbacks)), groups, kwargs)
        context_window = self.resolve(
            chain_of(kwargs.get("context_window_fallbacks", self.config.context_window_fallbacks)),
            groups,
            kwargs,
        )
        content_policy = self.resolve(
            chain_of(kwargs.get("content_policy_fallbacks", self.config.content_policy_fallbacks)),
            groups,
            kwargs,
        )
        disabled = fallbacks_disabled_for_request(kwargs)
        self.apply(kwargs)
        hop = self.hop(kwargs)
        snapshot = self.snapshot.kwargs()
        carry_over_pre_routing_selection(kwargs, snapshot)
        return Output(
            lookup_groups=list(groups),
            ordinary=ordinary,
            context_window=context_window,
            content_policy=content_policy,
            disabled=disabled,
            budget=self.budget(),
            metadata=bucket_output(kwargs.get("metadata")),
            litellm_metadata=bucket_output(kwargs.get("litellm_metadata")),
            carried=get_pre_routing_selection(snapshot),
            hop=hop,
        )

    def scenario(self) -> dict[str, object]:
        # the Router validates the configured chains the way a real router does, then the probe resolves them itself
        _router = Router(
            model_list=[],
            fallbacks=self.config.fallbacks,
            context_window_fallbacks=self.config.context_window_fallbacks,
            content_policy_fallbacks=self.config.content_policy_fallbacks,
        )
        kwargs = self.state.kwargs()
        kwargs["attempted_targets"] = attempt_targets(self.attempted)
        if self.request.fallbacks is not None:
            kwargs["fallbacks"] = self.request.fallbacks
        if self.request.context_window_fallbacks is not None:
            kwargs["context_window_fallbacks"] = self.request.context_window_fallbacks
        if self.request.content_policy_fallbacks is not None:
            kwargs["content_policy_fallbacks"] = self.request.content_policy_fallbacks
        if self.request.num_retries is not None:
            kwargs["num_retries"] = self.request.num_retries
        if self.request.model_group_retry_policy is not None:
            kwargs["model_group_retry_policy"] = self.request.model_group_retry_policy
        return kwargs

    def resolve(
        self,
        fallbacks: list[JsonValue] | None,
        groups: tuple[str, ...],
        kwargs: dict[str, object],
    ) -> ChainOutput:
        if fallbacks is None:
            return ChainOutput(resolved=False, targets=[], has_unattempted=False)
        resolved, _generic = get_fallback_model_group_for_lookup_groups(fallbacks=fallbacks, lookup_groups=groups)
        if resolved is None:
            return ChainOutput(resolved=False, targets=[], has_unattempted=False)
        return ChainOutput(
            resolved=True,
            targets=cast("list[JsonValue]", resolved),
            has_unattempted=has_unattempted_fallback_target(resolved, kwargs),
        )

    def apply(self, kwargs: dict[str, object]) -> None:
        if self.record_selection is not None:
            clear_pre_routing_selection(kwargs)
            record_pre_routing_selection(kwargs, self.record_selection)
        if self.record_disable is not None:
            record_disable_fallbacks(kwargs, self.record_disable)

    def budget(self) -> BudgetOutput:
        return BudgetOutput(
            depth=self.config.depth,
            max_fallbacks=self.config.max_fallbacks,
            exhausted=self.config.depth >= self.config.max_fallbacks,
        )

    def hop(self, kwargs: dict[str, object]) -> HopOutput:
        controls = per_request_fallback_controls(kwargs)
        hop = mid_stream_fallback_hop_kwargs(self.state.model or "", probe_function, controls, kwargs)
        return HopOutput(
            model=cast("str", hop.get("model", "")),
            fallbacks=hop_field(hop, "fallbacks"),
            context_window_fallbacks=hop_field(hop, "context_window_fallbacks"),
            content_policy_fallbacks=hop_field(hop, "content_policy_fallbacks"),
            num_retries=cast("int | None", hop.get("num_retries")),
            model_group_retry_policy=cast("JsonValue | None", hop.get("model_group_retry_policy")),
        )
