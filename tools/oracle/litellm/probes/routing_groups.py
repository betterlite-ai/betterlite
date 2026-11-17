import litellm
from litellm.router import Router
from litellm.router_utils.reasoning_effort_capability import (
    declared_reasoning_efforts_for_model,
    deployment_is_catalog_mapped,
    intersect_supported_reasoning_efforts,
    nearest_declared_reasoning_effort,
    resolve_supported_reasoning_efforts,
)
from litellm.router_utils.routing_groups import parse_routing_groups
from litellm.types.router import RoutingGroup
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, Strict


class Group(Strict):
    group_name: str
    models: list[str]
    routing_strategy: str
    routing_strategy_args: dict[str, JsonValue] | None
    model_priorities: dict[str, int] | None


GROUP: TypeAdapter[Group] = TypeAdapter(Group)


class Deployment(Strict):
    model_name: str
    identifier: str
    model: str


class Effort(Strict):
    model_info: dict[str, JsonValue]
    resolved_model_info: dict[str, JsonValue] | None = None


class Intersection(Strict):
    current: list[str] | None = None
    resolved: list[str] | None = None


class Round(Strict):
    requested: str
    declared: list[str]


class Lookup(Strict):
    model: str
    custom_llm_provider: str


class Name(Strict):
    name: str
    is_callable: bool
    priority: bool
    alternatives: bool
    ids: list[str] | None


class Output(Strict):
    groups: list[Group]
    error: str | None
    names: list[Name]
    efforts: list[list[str] | None]
    intersections: list[list[str] | None]
    rounds: list[str]
    declared: list[list[str] | None]


def effort_result(case: Effort) -> list[str] | None:
    mapped = deployment_is_catalog_mapped(case.resolved_model_info, case.model_info)
    resolved = resolve_supported_reasoning_efforts(case.model_info, deployment_is_mapped=mapped)
    return None if resolved is None else list(resolved)


def intersection_result(case: Intersection) -> list[str] | None:
    resolved = intersect_supported_reasoning_efforts(case.current, case.resolved)
    return None if resolved is None else list(resolved)


def round_result(case: Round) -> str:
    return nearest_declared_reasoning_effort(
        case.requested,
        case.declared,
    )


def declared_result(lookup: Lookup) -> list[str] | None:
    declared = declared_reasoning_efforts_for_model(lookup.model, lookup.custom_llm_provider)
    return None if declared is None else list(declared)


def name_facts(router: Router, name: str) -> Name:
    group = router.get_routing_group(name)
    ids: list[str] | None = None
    if group is not None:
        ids = sorted(router.get_candidate_model_ids_for_route(name))
    return Name(
        name=name,
        is_callable=group is not None,
        priority=group is not None and group.routing_strategy == "priority",
        alternatives=router.routing_group_has_alternatives(name),
        ids=ids,
    )


class RoutingGroups(Operation):
    groups: list[dict[str, JsonValue]] | None = None
    deployments: list[Deployment] = Field(default_factory=list)
    names: list[str] = Field(default_factory=list)
    cost_map: dict[str, JsonValue] = Field(default_factory=dict)
    efforts: list[Effort] = Field(default_factory=list)
    intersections: list[Intersection] = Field(default_factory=list)
    rounds: list[Round] = Field(default_factory=list)
    lookups: list[Lookup] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        if len(self.cost_map) > 0:
            litellm.model_cost = dict(self.cost_map)
        efforts = [effort_result(case) for case in self.efforts]
        intersections = [intersection_result(case) for case in self.intersections]
        rounds = [round_result(case) for case in self.rounds]
        declared = [declared_result(lookup) for lookup in self.lookups]
        groups: list[Group] = []
        names: list[Name] = []
        error: str | None = None
        try:
            router = self.build()
            groups = [GROUP.validate_json(group.model_dump_json()) for group in self.parse()]
            names = [name_facts(router, name) for name in self.names]
        except ValueError as failure:
            error = str(failure)
        return Output(
            groups=groups,
            error=error,
            names=names,
            efforts=efforts,
            intersections=intersections,
            rounds=rounds,
            declared=declared,
        )

    def parse(self) -> tuple[RoutingGroup, ...]:
        raw = self.groups if self.groups is not None else []
        known = frozenset(deployment.model_name for deployment in self.deployments)
        return parse_routing_groups(raw, known_model_names=known)

    def build(self) -> Router:
        entries: list[dict[str, JsonValue]] = [
            {
                "model_name": deployment.model_name,
                "litellm_params": {"model": deployment.model},
                "model_info": {"id": deployment.identifier},
            }
            for deployment in self.deployments
        ]
        raw = self.groups if self.groups is not None else []
        groups: list[RoutingGroup | dict[str, JsonValue]] = []
        groups.extend(raw)
        return Router(model_list=entries, routing_groups=groups)
