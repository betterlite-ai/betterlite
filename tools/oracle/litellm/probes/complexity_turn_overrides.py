import asyncio
import warnings
from collections.abc import Mapping, Sequence
from typing import Final
from unittest.mock import patch

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from litellm.router_strategy.complexity_router.config import ComplexityTier
from litellm.router_strategy.complexity_router.stall_detector import detect_stalled_task
from litellm.types.utils import Embedding, EmbeddingResponse
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false, reportUnknownMemberType=false

MODEL_NAME: Final = "complexity-turn-overrides"


class RuleInput(Strict):
    keywords: list[str]
    tier: str


class MarkerInput(Strict):
    open: str
    close: str


class CallOut(Strict):
    stage: str
    model: str
    inputs: list[str]


class OverrideOut(Strict):
    tier: str
    matched_keyword: str | None


class Output(Strict):
    plan_mode_sentinel: str | None
    plan_mode_floor: str | None
    plan_mode_floor_is_top_tier: bool
    applied_plan_mode_floor: str | None
    escalation_keyword: str | None
    stalled: bool
    housekeeping_sentinel: str | None
    housekeeping_tier: str | None
    keyword_override: OverrideOut | None
    embedding_calls: list[CallOut]


class DocumentArgs(Strict):
    model: str
    input: list[str]


class QueryArgs(Strict):
    model: str
    input: list[str]
    metadata: dict[str, JsonValue] = Field(default_factory=dict)
    litellm_metadata: dict[str, JsonValue] = Field(default_factory=dict)
    proxy_server_request: dict[str, JsonValue] = Field(default_factory=dict)
    turn_off_message_logging: bool | None = None
    litellm_session_id: str | None = None
    litellm_trace_id: str | None = None


def tier_name(value: ComplexityTier | str | None) -> str | None:
    if value is None:
        return None
    return value.value if isinstance(value, ComplexityTier) else value


class Provider:
    def __init__(self, payload: "ComplexityTurnOverrides") -> None:
        self.vectors: dict[str, list[float]] = payload.embeddings
        self.document_failures: int = payload.document_failures
        self.query_failures: int = payload.query_failures
        self.calls: list[CallOut] = []

    def response(self, model: str, inputs: list[str]) -> EmbeddingResponse:
        data = [
            Embedding(embedding=self.vectors[text], index=index, object="embedding")
            for index, text in enumerate(inputs)
        ]
        return EmbeddingResponse(model=model, data=data)

    def embedding(self, *_args: object, **kwargs: object) -> EmbeddingResponse:
        args = DocumentArgs.model_validate(kwargs)
        self.calls.append(CallOut(stage="documents", model=args.model, inputs=args.input))
        if self.document_failures > 0:
            self.document_failures -= 1
            raise ValueError("the probe failed the document embedding")
        return self.response(args.model, args.input)

    async def aembedding(self, *_args: object, **kwargs: object) -> EmbeddingResponse:
        args = QueryArgs.model_validate(kwargs)
        self.calls.append(CallOut(stage="query", model=args.model, inputs=args.input))
        if self.query_failures > 0:
            self.query_failures -= 1
            raise ValueError("the probe failed the query embedding")
        return self.response(args.model, args.input)


class ProbeRouter(ComplexityRouter):
    async def keyword_override(
        self, user_message: str, request_kwargs: dict[str, JsonValue]
    ) -> OverrideOut | None:
        found = await self._resolve_keyword_tier_override(user_message, request_kwargs)
        if found is None:
            return None
        return OverrideOut(
            tier=tier_name(found.tier) or "", matched_keyword=found.matched_keyword
        )

    def plan_sentinel(
        self,
        request_kwargs: Mapping[str, object],
        messages: Sequence[Mapping[str, object]] | None,
    ) -> str | None:
        return self._matched_plan_mode_signal(request_kwargs, messages)

    def housekeeping(self, newest_ask: str | None) -> tuple[str | None, str | None]:
        sentinel = self._matched_housekeeping_sentinel(newest_ask)
        cheapest = self._cheapest_configured_tier() if sentinel is not None else None
        return sentinel, tier_name(cheapest)

    def floor(self) -> str | None:
        return tier_name(self._resolve_plan_mode_floor())

    def floor_is_top(self) -> bool:
        return self._plan_mode_floor_is_top_tier()

    def applied_tier(self, decided: str | None) -> str | None:
        if decided is None:
            return None
        tier = decided if self.config.has_custom_tiers else ComplexityTier(decided)
        return tier_name(self._apply_plan_mode_floor(tier))

    def escalation(self, newest_ask: str | None) -> str | None:
        if newest_ask is None:
            return None
        return self._matched_escalation_keyword(newest_ask)


class ComplexityTurnOverrides(Operation):
    tiers: dict[str, list[str]] = Field(default_factory=dict)
    tier_definitions: list[str] | None = None
    non_reasoning: bool = False
    keyword_tier_rules: list[RuleInput] = Field(default_factory=list)
    semantic_keyword_matching: bool = False
    embedding_model: str | None = None
    match_threshold: float = 0.5
    stall_escalation_enabled: bool = False
    stall_escalation_window: int = 6
    stall_escalation_repeat_threshold: int = 3
    route_housekeeping_to_cheapest_tier: bool = True
    escalation_keywords: list[str] | None = None
    plan_mode_min_tier: str | None = None
    plan_mode_patterns: list[str] = Field(default_factory=list)
    housekeeping_patterns: list[str] = Field(default_factory=list)
    reminder_markers: list[MarkerInput] | None = None
    classifier_type: str | None = None
    messages: list[JsonValue] = Field(default_factory=list)
    request_kwargs: dict[str, JsonValue] | None = None
    user_message: str = ""
    newest_ask: str | None = None
    decided_tier: str | None = None
    turns: int = 1
    embeddings: dict[str, list[float]] = Field(default_factory=dict)
    document_failures: int = 0
    query_failures: int = 0

    @override
    def run(self) -> Output:
        provider = Provider(self)
        # semantic_router's route sync calls pydantic's deprecated dict(); that is not compared behavior
        warnings.filterwarnings(
            "ignore",
            message="The `dict` method is deprecated",
            category=DeprecationWarning,
        )
        messages = self.raw_messages()
        request_kwargs: dict[str, JsonValue] = self.request_kwargs or {}
        with (
            patch.object(Router, "embedding", provider.embedding),
            patch.object(Router, "aembedding", provider.aembedding),
        ):
            router = self.router()
            override = asyncio.run(self.keyword_overrides(router, request_kwargs))
        sentinel, housekeeping_tier = router.housekeeping(self.newest_ask)
        stalled = self.stall_escalation_enabled and detect_stalled_task(
            messages or None,
            window=self.stall_escalation_window,
            repeat_threshold=self.stall_escalation_repeat_threshold,
        )
        return Output(
            plan_mode_sentinel=router.plan_sentinel(request_kwargs, messages or None),
            plan_mode_floor=router.floor(),
            plan_mode_floor_is_top_tier=router.floor_is_top(),
            applied_plan_mode_floor=router.applied_tier(self.decided_tier),
            escalation_keyword=router.escalation(self.newest_ask),
            stalled=stalled,
            housekeeping_sentinel=sentinel,
            housekeeping_tier=housekeeping_tier,
            keyword_override=override,
            embedding_calls=provider.calls,
        )

    async def keyword_overrides(
        self, router: ProbeRouter, request_kwargs: dict[str, JsonValue]
    ) -> OverrideOut | None:
        override: OverrideOut | None = None
        for _turn in range(self.turns):
            override = await router.keyword_override(self.user_message, request_kwargs)
        return override

    def router(self) -> ProbeRouter:
        return ProbeRouter(
            model_name=MODEL_NAME,
            litellm_router_instance=Router(),
            complexity_router_config=self.config(),
        )

    def config(self) -> Mapping[str, object]:
        return {
            "tiers": {name: list(models) for name, models in self.tiers.items()},
            "tier_definitions": self.tier_definitions_config(),
            "enable_non_reasoning_tier": self.non_reasoning,
            "keyword_tier_rules": [
                {"keywords": rule.keywords, "tier": rule.tier} for rule in self.keyword_tier_rules
            ]
            or None,
            "semantic_keyword_matching": self.semantic_keyword_matching,
            "embedding_model": self.embedding_model,
            "match_threshold": self.match_threshold,
            "stall_escalation_enabled": self.stall_escalation_enabled,
            "stall_escalation_window": self.stall_escalation_window,
            "stall_escalation_repeat_threshold": self.stall_escalation_repeat_threshold,
            "route_housekeeping_to_cheapest_tier": self.route_housekeeping_to_cheapest_tier,
            "escalation_keywords": self.escalation_keywords,
            "plan_mode_min_tier": self.plan_mode_min_tier,
            "plan_mode_patterns": list(self.plan_mode_patterns),
            "housekeeping_patterns": list(self.housekeeping_patterns),
            "reminder_markers": self.markers_config(),
            "classifier_type": self.classifier(),
            "fallback_tier": self.fallback_tier(),
        }

    def tier_definitions_config(self) -> list[JsonValue] | None:
        if self.tier_definitions is None:
            return None
        return [
            {"name": name, "description": f"probe tier {name}"} for name in self.tier_definitions
        ]

    def fallback_tier(self) -> str | None:
        if self.tier_definitions is None:
            return None
        return self.tier_definitions[0]

    def markers_config(self) -> list[JsonValue] | None:
        if self.reminder_markers is None:
            return None
        return [{"open": pair.open, "close": pair.close} for pair in self.reminder_markers]

    def classifier(self) -> str:
        if self.classifier_type is not None:
            return self.classifier_type
        return "custom" if self.tier_definitions is not None else "heuristic"

    def raw_messages(self) -> list[dict[str, JsonValue]]:
        found: list[dict[str, JsonValue]] = []
        for message in self.messages:
            if not isinstance(message, dict):
                raise ProbeError("invalid_request", "each message must be an object")
            found.append(message)
        return found
