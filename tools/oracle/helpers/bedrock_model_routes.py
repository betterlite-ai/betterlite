from typing import TypeAlias, cast

import litellm
from litellm.llms.bedrock.common_utils import (
    BedrockModelInfo,
    _openai_gpt_version,  # pyright: ignore[reportPrivateUsage]
    _response_format_needs_converse,  # pyright: ignore[reportPrivateUsage]
    bedrock_model_is_openai_gpt,
    bedrock_request_needs_converse,
    bedrock_route_for_request,
    bedrock_runtime_chat_completions_enforces_response_format,
    bedrock_runtime_chat_completions_is_default,
    bedrock_runtime_chat_completions_serves_tools_with_reasoning,
    get_bedrock_tool_name,
    without_bedrock_route_prefix,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return value


def strings(value: Json, name: str) -> list[str]:
    return [string_value(item, name) for item in list_value(value, name)]


def apply_fixtures(input_value: dict[str, Json]) -> None:
    costs = cast("dict[str, Json]", litellm.model_cost)
    costs.clear()
    costs.update(object_value(input_value.get("price_map", {}), "price_map"))
    converse = cast("set[str]", litellm.bedrock_converse_models)
    converse.clear()
    converse.update(strings(input_value.get("converse_models", []), "converse_models"))
    litellm.bedrock_request_metadata_fields = strings(
        input_value.get("request_metadata_fields", []), "request_metadata_fields"
    )
    tool_names = object_value(input_value.get("tool_names", {}), "tool_names")
    litellm.bedrock_tool_name_mappings.cache_dict = tool_names


def run_routes(step: dict[str, Json]) -> Json:
    models = strings(step.get("models", []), "models")
    plain = [[model, BedrockModelInfo.get_bedrock_route(model)] for model in models]
    requests: list[Json] = []
    for probe in list_value(step.get("requests", []), "requests"):
        item = object_value(probe, "requests")
        model = string_value(item.get("model"), "model")
        request_params = object_value(item.get("request_params", {}), "request_params")
        drops_value = item.get("additional_drop_params")
        drops = strings(drops_value, "additional_drop_params") if drops_value is not None else None
        before = dict(request_params)
        route = bedrock_route_for_request(model, request_params, drops)
        requests.append([model, route, request_params == before])
    return cast("Json", {"models": plain, "requests": requests})


def capability_facts(model: str) -> dict[str, Json]:
    version = _openai_gpt_version(model)
    serves_tools = bedrock_runtime_chat_completions_serves_tools_with_reasoning(model)
    enforces_format = bedrock_runtime_chat_completions_enforces_response_format(model)
    return {
        "version": None if version is None else [version[0], version[1]],
        "is_openai_gpt": bedrock_model_is_openai_gpt(model),
        "is_default": bedrock_runtime_chat_completions_is_default(model),
        "serves_tools_with_reasoning": serves_tools,
        "enforces_response_format": enforces_format,
    }


def probe_raises(model: str) -> bool:
    try:
        bedrock_runtime_chat_completions_is_default(model)
    except Exception:  # noqa: BLE001 - the probe records that the pinned call raised
        return True
    return False


def run_capabilities(step: dict[str, Json]) -> Json:
    models = strings(step.get("models", []), "models")
    malformed = strings(step.get("malformed", []), "malformed")
    return cast(
        "Json",
        {
            "models": [[model, capability_facts(model)] for model in models],
            "malformed": [[model, probe_raises(model)] for model in malformed],
        },
    )


def run_request_fallbacks(step: dict[str, Json]) -> Json:
    results: list[Json] = []
    for probe in list_value(step.get("probes", []), "probes"):
        item = object_value(probe, "probes")
        model = string_value(item.get("model"), "model")
        request_params = object_value(item.get("request_params", {}), "request_params")
        results.append([model, bedrock_request_needs_converse(model, request_params)])
    return cast("Json", results)


def run_response_formats(step: dict[str, Json]) -> Json:
    results: list[Json] = []
    for probe in list_value(step.get("probes", []), "probes"):
        item = object_value(probe, "probes")
        model = string_value(item.get("model"), "model")
        # the pinned helper takes any object and owns the dict/nondict split
        needed = _response_format_needs_converse(model, item.get("response_format"))
        results.append([model, needed])
    return cast("Json", results)


def run_tool_names(step: dict[str, Json]) -> Json:
    names = strings(step.get("names", []), "names")
    return cast("Json", [[name, get_bedrock_tool_name(name)] for name in names])


def prefix_facts(model: str) -> dict[str, Json]:
    return {
        "without_route_prefix": without_bedrock_route_prefix(model),
        "claude_platform_model": BedrockModelInfo.get_claude_platform_model(model),
        "explicit_converse": BedrockModelInfo._explicit_converse_route(model),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "explicit_claude_platform": BedrockModelInfo._explicit_claude_platform_route(model),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "explicit_mantle": BedrockModelInfo._explicit_mantle_route(model),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "chat_completions": BedrockModelInfo._model_has_route_prefix(model, "chat_completions/"),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    }


def workspace_facts(probe: Json) -> Json:
    item = object_value(probe, "workspace")
    passed = object_value(item.get("passed", {}), "passed")
    optional = dict(object_value(item.get("optional", {}), "optional"))
    mapped = BedrockModelInfo.map_claude_platform_auth_params(passed, optional)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    return cast("Json", mapped)


def run_prefixes(step: dict[str, Json]) -> Json:
    models = strings(step.get("models", []), "models")
    probes = list_value(step.get("workspace", []), "workspace")
    workspace = [workspace_facts(probe) for probe in probes]
    return cast(
        "Json",
        {"models": [[model, prefix_facts(model)] for model in models], "workspace_keys": workspace},
    )


def run_model_info(step: dict[str, Json]) -> Json:
    results: list[Json] = []
    for probe in list_value(step.get("probes", []), "probes"):
        item = object_value(probe, "probes")
        api_base_value = item.get("api_base")
        api_key_value = item.get("api_key")
        api_base = None if api_base_value is None else string_value(api_base_value, "api_base")
        api_key = None if api_key_value is None else string_value(api_key_value, "api_key")
        headers = object_value(item.get("headers", {}), "headers")
        environment = BedrockModelInfo().validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            headers=headers, model="", messages=[], optional_params={}, litellm_params={}
        )
        results.append(
            cast(
                "Json",
                {
                    "api_base": BedrockModelInfo.get_api_base(api_base),
                    "api_key": BedrockModelInfo.get_api_key(api_key),
                    "headers": cast("Json", environment),
                    "models": BedrockModelInfo().get_models(),
                },
            )
        )
    return cast("Json", results)


def run(input_value: dict[str, Json]) -> Json:
    apply_fixtures(input_value)
    step = object_value(input_value.get("step"), "step")
    match step:
        case {"routes": dict() as routes_step}:
            result = run_routes(routes_step)
        case {"capabilities": dict() as capabilities_step}:
            result = run_capabilities(capabilities_step)
        case {"request_needs_converse": dict() as probes_step}:
            result = run_request_fallbacks(probes_step)
        case {"response_format": dict() as probes_step}:
            result = run_response_formats(probes_step)
        case {"tool_names": dict() as names_step}:
            result = run_tool_names(names_step)
        case {"prefixes": dict() as prefixes_step}:
            result = run_prefixes(prefixes_step)
        case {"model_info": dict() as info_step}:
            result = run_model_info(info_step)
        case _:
            raise ValueError(f"unknown bedrock_model_routes step: {step}")
    return result
