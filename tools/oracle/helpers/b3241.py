from collections.abc import Callable
from typing import Final, Literal, TypeAlias, cast

import litellm
import litellm.proxy.policy_engine.attachment_registry as attachment_registry_module
import litellm.proxy.policy_engine.policy_registry as policy_registry_module
from litellm.proxy.policy_engine.attachment_registry import AttachmentRegistry
from litellm.proxy.policy_engine.policy_matcher import PolicyMatcher
from litellm.proxy.policy_engine.policy_registry import PolicyRegistry
from litellm.proxy.policy_engine.policy_resolver import PolicyResolver
from litellm.types.proxy.policy_engine.pipeline_types import GuardrailPipeline
from litellm.types.proxy.policy_engine.policy_types import (
    Policy,
    PolicyAttachment,
    PolicyScope,
)
from litellm.types.proxy.policy_engine.resolver_types import PolicyMatchContext

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Handler: TypeAlias = Callable[[dict[str, Json]], Json]
PAIR_LENGTH: Final = 2


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def optional_list(value: Json, name: str) -> list[Json]:
    if value is None:
        return []
    return list_value(value, name)


def string_list(value: Json, name: str) -> list[str]:
    return [string_value(item, name) for item in list_value(value, name)]


def policy(value: Json, name: str) -> Policy:
    return Policy(**object_value(value, name))  # pyright: ignore[reportArgumentType]


def policy_context(value: Json) -> PolicyMatchContext:
    return PolicyMatchContext(**object_value(value, "context"))  # pyright: ignore[reportArgumentType]


def policy_scope(value: Json) -> PolicyScope:
    return PolicyScope(**object_value(value, "scope"))  # pyright: ignore[reportArgumentType]


def policy_attachment(value: Json) -> PolicyAttachment:
    return PolicyAttachment(**object_value(value, "attachment"))  # pyright: ignore[reportArgumentType]


def attachments_value(value: Json) -> list[PolicyAttachment]:
    return [policy_attachment(item) for item in list_value(value, "attachments")]


def attachment_dicts(value: Json) -> list[dict[str, Json]]:
    if value is None:
        return []
    return [object_value(item, "attachment") for item in list_value(value, "attachments")]


def policy_configs(value: Json) -> dict[str, dict[str, object]]:
    entries = object_value(value, "policies")
    configs: dict[str, dict[str, object]] = {}
    for name, entry in entries.items():
        configs[name] = cast("dict[str, object]", object_value(entry, name))
    return configs


def policy_json(policy_value: Policy) -> Json:
    return cast(Json, policy_value.model_dump(mode="json"))


def policy_json_or_none(policy_value: Policy | None) -> Json:
    if policy_value is None:
        return None
    return policy_json(policy_value)


def loaded_policies(value: Json) -> dict[str, Policy]:
    registry = PolicyRegistry()
    registry.load_policies(policy_configs(value))
    return registry.get_all_policies()


def registry_snapshot(registry: PolicyRegistry, removed: list[bool]) -> Json:
    policies = registry.get_all_policies()
    config_policies = registry.list_config_policies()
    return cast(
        Json,
        {
            "initialized": registry.is_initialized(),
            "removed": removed,
            "names": sorted(policies),
            "sources": {name: registry.get_source(name) for name in sorted(policies)},
            "policies": {name: policy_json(policies[name]) for name in sorted(policies)},
            "config_policies": {
                name: policy_json(config_policies[name]) for name in sorted(config_policies)
            },
        },
    )


def registry_load_policies(input_value: dict[str, Json]) -> Json:
    registry = PolicyRegistry()
    registry.load_policies(policy_configs(input_value.get("policies")))
    return registry_snapshot(registry, [])


def registry_parse_policy(input_value: dict[str, Json]) -> Json:
    name = string_value(input_value.get("policy_name"), "policy_name")
    registry = PolicyRegistry()
    registry.load_policies({name: policy_configs_for(input_value.get("policy"))})
    return cast(Json, {"policy": policy_json_or_none(registry.get_policy(name))})


def policy_configs_for(value: Json) -> dict[str, object]:
    return cast("dict[str, object]", object_value(value, "policy"))


def registry_parse_pipeline(input_value: dict[str, Json]) -> Json:
    definition = object_value(input_value.get("pipeline"), "pipeline")
    registry = PolicyRegistry()
    registry.load_policies({"pipeline-holder": {"pipeline": definition}})
    policy_value = registry.get_policy("pipeline-holder")
    if policy_value is None or policy_value.pipeline is None:
        raise ValueError("pipeline must be an object")
    return cast(Json, {"pipeline": policy_value.pipeline.model_dump(mode="json")})


def registry_get_policy(input_value: dict[str, Json]) -> Json:
    registry = PolicyRegistry()
    registry.load_policies(policy_configs(input_value.get("policies")))
    found: dict[str, Json] = {}
    for name in string_list(input_value.get("policy_names"), "policy_names"):
        found[name] = policy_json_or_none(registry.get_policy(name))
    return cast(Json, {"policies": found})


def registry_has_policy(input_value: dict[str, Json]) -> Json:
    registry = PolicyRegistry()
    registry.load_policies(policy_configs(input_value.get("policies")))
    present = {
        name: registry.has_policy(name)
        for name in string_list(input_value.get("policy_names"), "policy_names")
    }
    return cast(Json, {"has": present})


def policy_source(value: Json) -> Literal["db", "config"]:
    source = string_value(value, "source")
    if source not in ("db", "config"):
        raise ValueError("source must be db or config")
    return source


def registry_scenario(input_value: dict[str, Json]) -> Json:
    registry = PolicyRegistry()
    policies_value = input_value.get("policies")
    if policies_value is not None:
        registry.load_policies(policy_configs(policies_value))
    for entry in optional_list(input_value.get("add"), "add"):
        item = object_value(entry, "add")
        registry.add_policy(
            string_value(item.get("name"), "name"),
            policy(item.get("policy"), "policy"),
            policy_source(item.get("source", "db")),
        )
    removed = [
        registry.remove_policy(name)
        for name in string_list(input_value.get("remove", []), "remove")
    ]
    return registry_snapshot(registry, removed)


def attachment_load(input_value: dict[str, Json]) -> Json:
    registry = AttachmentRegistry()
    registry.load_attachments(attachment_dicts(input_value.get("attachments")))  # pyright: ignore[reportArgumentType]
    return cast(
        Json,
        {
            "initialized": registry.is_initialized(),
            "attachments": [
                item.model_dump(mode="json") for item in registry.get_all_attachments()
            ],
            "config_attachments": [
                item.model_dump(mode="json") for item in registry.get_config_attachments()
            ],
        },
    )


def attachment_parse(input_value: dict[str, Json]) -> Json:
    attachment = policy_attachment(input_value.get("attachment"))
    return cast(Json, {"attachment": attachment.model_dump(mode="json")})


def attachment_specificity(input_value: dict[str, Json]) -> Json:
    values = [
        list(attachment_registry_module._attachment_specificity(item))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        for item in attachments_value(input_value.get("attachments"))
    ]
    return cast(Json, {"specificity": values})


def attachment_sort_key(input_value: dict[str, Json]) -> Json:
    keys = [
        list(attachment_registry_module._attachment_sort_key(item))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        for item in attachments_value(input_value.get("attachments"))
    ]
    return cast(Json, {"sort_keys": keys})


def attachment_attached(input_value: dict[str, Json]) -> Json:
    registry = AttachmentRegistry()
    registry.load_attachments(attachment_dicts(input_value.get("attachments")))  # pyright: ignore[reportArgumentType]
    context = policy_context(input_value.get("context"))
    policies_value = input_value.get("policies")
    predicate: Callable[[str], bool] | None = None
    if policies_value is not None:
        predicate = PolicyMatcher.policy_applies(context, loaded_policies(policies_value))
    names = registry.get_attached_policies(context, predicate)
    matches = registry.get_attached_policies_with_reasons(context, predicate)
    return cast(Json, {"policies": names, "matches": matches})


def install_registries(policies_value: Json, attachments: Json) -> None:
    policies = PolicyRegistry()
    policies.load_policies(policy_configs(policies_value))
    attachment_registry = AttachmentRegistry()
    attachment_registry.load_attachments(attachment_dicts(attachments))  # pyright: ignore[reportArgumentType]
    policy_registry_module._policy_registry = policies  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    attachment_registry_module._attachment_registry = attachment_registry  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def resolver_guardrails(input_value: dict[str, Json]) -> Json:
    install_registries(input_value.get("policies"), input_value.get("attachments"))
    names_value = input_value.get("policy_names")
    names = None if names_value is None else string_list(names_value, "policy_names")
    guardrails = PolicyResolver.resolve_guardrails_for_context(
        context=policy_context(input_value.get("context")),
        policy_names=names,
    )
    return cast(Json, {"guardrails": sorted(guardrails)})


def resolver_pipelines(input_value: dict[str, Json]) -> Json:
    install_registries(input_value.get("policies"), input_value.get("attachments"))
    names_value = input_value.get("policy_names")
    names = None if names_value is None else string_list(names_value, "policy_names")
    pipelines = PolicyResolver.resolve_pipelines_for_context(
        context=policy_context(input_value.get("context")),
        policy_names=names,
    )
    return cast(
        Json,
        {"pipelines": [[name, item.model_dump(mode="json")] for name, item in pipelines]},
    )


def pipeline_pairs(value: Json) -> list[tuple[str, GuardrailPipeline]]:
    pairs: list[tuple[str, GuardrailPipeline]] = []
    for entry in list_value(value, "pipelines"):
        pair = list_value(entry, "pipeline")
        if len(pair) != PAIR_LENGTH:
            raise ValueError("pipelines must hold [policy_name, pipeline] pairs")
        name = string_value(pair[0], "policy_name")
        definition = object_value(pair[1], "pipeline")
        pairs.append((name, GuardrailPipeline(**definition)))  # pyright: ignore[reportArgumentType]
    return pairs


def resolver_managed(input_value: dict[str, Json]) -> Json:
    managed = PolicyResolver.get_pipeline_managed_guardrails(
        pipeline_pairs(input_value.get("pipelines"))
    )
    return cast(Json, {"guardrails": sorted(managed)})


def matcher_matches_pattern(input_value: dict[str, Json]) -> Json:
    patterns = string_list(input_value.get("patterns"), "patterns")
    value = input_value.get("value")
    if value is None:
        return cast(Json, {"matches": PolicyMatcher.matches_pattern(None, patterns)})
    present = string_value(value, "value")
    return cast(Json, {"matches": PolicyMatcher.matches_pattern(present, patterns)})


def matcher_scope_matches(input_value: dict[str, Json]) -> Json:
    matches = PolicyMatcher.scope_matches(
        scope=policy_scope(input_value.get("scope")),
        context=policy_context(input_value.get("context")),
    )
    return cast(Json, {"matches": matches})


def matcher_policy_applies(input_value: dict[str, Json]) -> Json:
    context = policy_context(input_value.get("context"))
    predicate = PolicyMatcher.policy_applies(context, loaded_policies(input_value.get("policies")))
    applies = {
        name: predicate(name)
        for name in string_list(input_value.get("policy_names"), "policy_names")
    }
    return cast(Json, {"applies": applies})


def matcher_chain(input_value: dict[str, Json]) -> Json:
    context = policy_context(input_value.get("context"))
    policies = loaded_policies(input_value.get("policies"))
    name = string_value(input_value.get("policy_name"), "policy_name")
    members = PolicyMatcher._applying_chain_members(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        policy_name=name, context=context, policies=policies
    )
    applies = PolicyMatcher.policy_applies(context, policies)(name)
    return cast(Json, {"members": list(members), "applies": applies})


def matcher_conditions(input_value: dict[str, Json]) -> Json:
    names = PolicyMatcher.get_policies_with_matching_conditions(
        policy_names=string_list(input_value.get("policy_names"), "policy_names"),
        context=policy_context(input_value.get("context")),
        policies=loaded_policies(input_value.get("policies")),
    )
    return cast(Json, {"names": list(names)})


HANDLERS: dict[str, dict[str, Handler]] = {
    "policy_memory_matching_registry": {
        "load_policies": registry_load_policies,
        "parse_policy": registry_parse_policy,
        "parse_pipeline": registry_parse_pipeline,
        "get_policy": registry_get_policy,
        "has_policy": registry_has_policy,
        "scenario": registry_scenario,
    },
    "policy_memory_matching_attachment": {
        "load_attachments": attachment_load,
        "parse_attachment": attachment_parse,
        "specificity": attachment_specificity,
        "sort_key": attachment_sort_key,
        "attached_policies": attachment_attached,
    },
    "policy_memory_matching_matcher": {
        "matches_pattern": matcher_matches_pattern,
        "scope_matches": matcher_scope_matches,
        "policy_applies": matcher_policy_applies,
        "applying_chain_members": matcher_chain,
        "matching_conditions": matcher_conditions,
    },
    "policy_memory_matching_resolver": {
        "resolve_guardrails": resolver_guardrails,
        "resolve_pipelines": resolver_pipelines,
        "managed_guardrails": resolver_managed,
    },
}


def only_group(input_value: dict[str, Json]) -> str:
    groups = list(input_value)
    if len(groups) != 1:
        raise ValueError("helper input must name exactly one group")
    return groups[0]


def run(input_value: dict[str, Json]) -> Json:
    group_name = only_group(input_value)
    group = HANDLERS.get(group_name)
    if group is None:
        raise ValueError(f"unknown {litellm.__name__} helper group {group_name}")
    payload = object_value(input_value[group_name], group_name)
    action = string_value(payload.get("action"), "action")
    handler = group.get(action)
    if handler is None:
        raise ValueError(f"unknown {litellm.__name__} helper action {group_name}.{action}")
    return handler(payload)
