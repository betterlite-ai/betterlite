from typing import Final, Literal, TypedDict

from litellm.proxy.common_utils.cache_aware_routing import eligible_models
from litellm.router_strategy.complexity_router.config import ComplexityRouterConfig
from litellm.types.router import RoutingContext, RoutingPlugin
from litellm.types.utils import StandardLoggingRoutingDecision
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

DECISION: Final[TypeAdapter[StandardLoggingRoutingDecision]] = TypeAdapter(
    StandardLoggingRoutingDecision
)


class TierConfig(TypedDict, total=False):
    tiers: dict[str, JsonValue]
    tier_definitions: list[dict[str, str]]
    enable_non_reasoning_tier: bool
    classifier_type: str
    fallback_tier: str
    adaptive: bool
    session_affinity: bool
    plugins: list[JsonValue]
    classification_mode: Literal["every_request", "user_turn"]


class Decision(TypedDict, total=False):
    tier: str
    cause: str


class Input(TypedDict):
    config: TierConfig
    decision: Decision


# a plugin for the truthiness check only; the plugin pipeline itself is not under test
class Plugin(RoutingPlugin):
    @override
    async def run(self, context: RoutingContext) -> RoutingContext:
        return context


def run(input_value: Input) -> dict[str, list[list[str]]]:
    raw = dict(input_value["config"])
    configured_plugins = bool(raw.pop("plugins", None))
    config = ComplexityRouterConfig.model_validate(raw)
    if configured_plugins:
        plugins: list[RoutingPlugin] = [Plugin()]
        config.plugins = plugins
    decision = DECISION.validate_python(input_value["decision"])
    eligible: list[list[str]] = [[tier, model] for tier, model in eligible_models(config, decision)]
    return {"eligible": eligible}
