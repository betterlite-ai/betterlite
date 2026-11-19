from collections.abc import Callable
from typing import Literal

from litellm.router_utils.auto_router_model_naming import (
    carries_complexity_router_settings,
    classify_strategy_router_model,
    defines_custom_classifier_prompt,
    defines_custom_tiers,
    strategy_router_dependencies,
    uses_custom_tier_or_classifier_prompt,
    uses_heuristic_v2_classifier,
    validate_complexity_router_config_placement,
    validate_complexity_router_config_write,
    validate_strategy_router_model_write,
)
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

CheckName = Literal[
    "classify",
    "dependencies",
    "predicates",
    "carries",
    "model_write",
    "placement",
    "config_write",
]


class Dependency(Strict):
    model_name: str
    role: str


class Output(Strict):
    check: CheckName
    kind: str | None = None
    dependencies: list[Dependency] = Field(default_factory=list)
    heuristic_v2: bool | None = None
    custom_tiers: bool | None = None
    custom_prompt: bool | None = None
    custom_tier_or_prompt: bool | None = None
    carries: bool | None = None
    model_write_error: str | None = None
    placement_error: str | None = None
    config_write_accepted: bool | None = None


class RouterDependencies(Operation):
    check: CheckName
    model: str | None = None
    litellm_params: dict[str, JsonValue] = Field(default_factory=dict)
    present_fields: list[str] = Field(default_factory=list)
    complexity_router_config: dict[str, JsonValue] | None = None

    @override
    def run(self) -> Output:
        builders: dict[CheckName, Callable[[], Output]] = {
            "classify": self._classify,
            "dependencies": self._dependencies,
            "predicates": self._predicates,
            "carries": self._carries,
            "model_write": self._model_write,
            "placement": self._placement,
            "config_write": self._config_write,
        }
        return builders[self.check]()

    def _classify(self) -> Output:
        model = self.model if self.model is not None else ""
        return Output(check=self.check, kind=classify_strategy_router_model(model))

    def _dependencies(self) -> Output:
        listed = [
            Dependency(model_name=dependency.model_name, role=dependency.role)
            for dependency in strategy_router_dependencies(self.litellm_params)
        ]
        return Output(check=self.check, dependencies=sorted(listed, key=_dependency_order))

    def _predicates(self) -> Output:
        config = self.litellm_params.get("complexity_router_config")
        return Output(
            check=self.check,
            heuristic_v2=uses_heuristic_v2_classifier(config),
            custom_tiers=defines_custom_tiers(config),
            custom_prompt=defines_custom_classifier_prompt(config),
            custom_tier_or_prompt=uses_custom_tier_or_classifier_prompt(config),
        )

    def _carries(self) -> Output:
        return Output(
            check=self.check,
            carries=carries_complexity_router_settings(self.model, frozenset(self.present_fields)),
        )

    def _model_write(self) -> Output:
        model = self.model if self.model is not None else ""
        return Output(
            check=self.check,
            model_write_error=validate_strategy_router_model_write(model, frozenset(self.present_fields)),
        )

    def _placement(self) -> Output:
        return Output(
            check=self.check,
            placement_error=validate_complexity_router_config_placement(self.litellm_params),
        )

    def _config_write(self) -> Output:
        error = validate_complexity_router_config_write(self.complexity_router_config)
        return Output(check=self.check, config_write_accepted=error is None)


def _dependency_order(dependency: Dependency) -> tuple[str, str]:
    return (dependency.role, dependency.model_name)
