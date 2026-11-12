import os
from collections.abc import Iterable, Mapping
from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.integrations.otel.model.config import ExporterSpec, OpenTelemetryV2Config
from litellm.integrations.otel.model.destination import OtelDestination
from litellm.integrations.otel.presets import (
    PRESET_BY_CALLBACK,
    dynamic_otlp_endpoint,
    dynamic_otlp_headers,
    project_routing_headers,
)
from litellm.integrations.otel.presets.agentops import agentops_preset
from litellm.integrations.otel.presets.destinations import (
    destination_capable_backends,
    destination_for,
)
from litellm.integrations.otel.presets.langfuse import langfuse_dynamic_headers, langfuse_preset
from litellm.integrations.otel.presets.newrelic import (
    newrelic_dynamic_endpoint,
    newrelic_dynamic_headers,
    newrelic_preset,
)
from litellm.integrations.otel.presets.signoz import (
    signoz_dynamic_endpoint,
    signoz_dynamic_headers,
    signoz_preset,
)
from litellm.types.utils import StandardCallbackDynamicParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Environment: TypeAlias = dict[str, str]


class ExporterInput(TypedDict):
    kind: NotRequired[str]
    endpoint: NotRequired[str | None]
    headers: NotRequired[str | None]
    requires_headers: NotRequired[bool]


class BaseInput(TypedDict):
    exporters: list[ExporterInput]
    mapper_names: list[str]
    resource_attributes: dict[str, str]


class AgentopsPresetInput(TypedDict):
    function: Literal["agentops_preset"]
    environment: Environment
    base: BaseInput


class NewrelicPresetInput(TypedDict):
    function: Literal["newrelic_preset"]
    environment: Environment
    base: BaseInput


class SignozPresetInput(TypedDict):
    function: Literal["signoz_preset"]
    environment: Environment
    base: BaseInput


class LangfusePresetInput(TypedDict):
    function: Literal["langfuse_preset"]
    environment: Environment
    base: BaseInput
    allow_missing_credentials: bool


class NewrelicDynamicHeadersInput(TypedDict):
    function: Literal["newrelic_dynamic_headers"]
    params: StandardCallbackDynamicParams


class NewrelicDynamicEndpointInput(TypedDict):
    function: Literal["newrelic_dynamic_endpoint"]
    params: StandardCallbackDynamicParams


class SignozDynamicHeadersInput(TypedDict):
    function: Literal["signoz_dynamic_headers"]
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class SignozDynamicEndpointInput(TypedDict):
    function: Literal["signoz_dynamic_endpoint"]
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class LangfuseDynamicHeadersInput(TypedDict):
    function: Literal["langfuse_dynamic_headers"]
    params: StandardCallbackDynamicParams


class PresetForInput(TypedDict):
    function: Literal["preset_for"]
    callback_name: str


class DynamicOtlpHeadersInput(TypedDict):
    function: Literal["dynamic_otlp_headers"]
    callback_name: str | None
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class DynamicOtlpEndpointInput(TypedDict):
    function: Literal["dynamic_otlp_endpoint"]
    callback_name: str | None
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class ProjectRoutingHeadersInput(TypedDict):
    function: Literal["project_routing_headers"]
    callback_name: str | None
    auth_metadata: dict[str, str] | None


class BackendsInput(TypedDict):
    function: Literal["destination_capable_backends"]


class DestinationForInput(TypedDict):
    function: Literal["destination_for"]
    environment: Environment
    allowed_hosts: list[str]
    callback_name: str
    params: StandardCallbackDynamicParams
    service_name: str | None


Input: TypeAlias = (
    AgentopsPresetInput
    | NewrelicPresetInput
    | SignozPresetInput
    | LangfusePresetInput
    | NewrelicDynamicHeadersInput
    | NewrelicDynamicEndpointInput
    | SignozDynamicHeadersInput
    | SignozDynamicEndpointInput
    | LangfuseDynamicHeadersInput
    | PresetForInput
    | DynamicOtlpHeadersInput
    | DynamicOtlpEndpointInput
    | ProjectRoutingHeadersInput
    | BackendsInput
    | DestinationForInput
)

_PRESET_FUNCTIONS = frozenset(
    {"agentops_preset", "newrelic_preset", "signoz_preset", "langfuse_preset"}
)
_BUILDER_FUNCTIONS = frozenset(
    {
        "newrelic_dynamic_headers",
        "newrelic_dynamic_endpoint",
        "signoz_dynamic_headers",
        "signoz_dynamic_endpoint",
        "langfuse_dynamic_headers",
    }
)
_ROUTING_FUNCTIONS = frozenset(
    {"dynamic_otlp_headers", "dynamic_otlp_endpoint", "project_routing_headers"}
)
_REGISTRY_FUNCTIONS = frozenset({"preset_for", "destination_capable_backends"})


def apply_environment(environment: Environment) -> None:
    os.environ.clear()
    os.environ.update(environment)


def string_map(values: Mapping[str, str]) -> dict[str, Json]:
    result: dict[str, Json] = {}
    result.update(values.items())
    return result


def strings_json(values: Iterable[str]) -> list[Json]:
    result: list[Json] = []
    result.extend(values)
    return result


def base_config(fields: BaseInput) -> OpenTelemetryV2Config:
    config = OpenTelemetryV2Config()
    config.exporters = [ExporterSpec.model_validate(item) for item in fields["exporters"]]
    config.mapper_names = list(fields["mapper_names"])
    config.resource_attributes = dict(fields["resource_attributes"])
    return config


def exporter_json(spec: ExporterSpec) -> Fields:
    options: dict[str, Json] | None = None
    if spec.options is not None:
        options = string_map(spec.options)
    owner: str | None = None
    if spec.owner is not None:
        owner = spec.owner.value
    return {
        "kind": spec.kind,
        "endpoint": spec.endpoint,
        "traces_endpoint": spec.traces_endpoint,
        "headers": spec.headers,
        "owner": owner,
        "options": options,
        "use_simple_processor": spec.use_simple_processor,
        "requires_headers": spec.requires_headers,
    }


def preset_output(config: OpenTelemetryV2Config) -> Fields:
    return {
        "exporters": [exporter_json(spec) for spec in config.exporters],
        "mapper_names": strings_json(config.mapper_names),
        "resource_attributes": string_map(config.resource_attributes),
        "service_name": config.service_name,
        "deployment_environment": config.deployment_environment,
        "capture_message_content": config.capture_message_content,
    }


def destination_json(destination: OtelDestination) -> Fields:
    return {
        "endpoint": destination.endpoint,
        "headers": string_map(destination.headers),
        "resource_attributes": string_map(destination.resource_attributes),
        "callback_name": destination.callback_name,
        "protocol": destination.protocol,
        "span_scope": destination.span_scope,
    }


def optional_headers(headers: Mapping[str, str] | None) -> Json:
    if headers is None:
        return None
    return string_map(headers)


def run_preset(input_value: Input) -> Json:
    if input_value["function"] == "agentops_preset":
        apply_environment(input_value["environment"])
        return preset_output(agentops_preset(config_overrides=base_config(input_value["base"])))
    if input_value["function"] == "newrelic_preset":
        apply_environment(input_value["environment"])
        return preset_output(newrelic_preset(config_overrides=base_config(input_value["base"])))
    if input_value["function"] == "signoz_preset":
        apply_environment(input_value["environment"])
        return preset_output(signoz_preset(config_overrides=base_config(input_value["base"])))
    if input_value["function"] == "langfuse_preset":
        apply_environment(input_value["environment"])
        return preset_output(
            langfuse_preset(
                config_overrides=base_config(input_value["base"]),
                allow_missing_credentials=input_value["allow_missing_credentials"],
            )
        )
    raise ValueError(f"unknown preset function {input_value['function']}")


def run_builder(input_value: Input) -> Json:
    if input_value["function"] == "newrelic_dynamic_headers":
        return string_map(newrelic_dynamic_headers(input_value["params"]))
    if input_value["function"] == "newrelic_dynamic_endpoint":
        return newrelic_dynamic_endpoint(input_value["params"])
    if input_value["function"] == "signoz_dynamic_headers":
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        return string_map(signoz_dynamic_headers(input_value["params"]))
    if input_value["function"] == "signoz_dynamic_endpoint":
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        return signoz_dynamic_endpoint(input_value["params"])
    if input_value["function"] == "langfuse_dynamic_headers":
        return string_map(langfuse_dynamic_headers(input_value["params"]))
    raise ValueError(f"unknown builder function {input_value['function']}")


def run_routing(input_value: Input) -> Json:
    if input_value["function"] == "dynamic_otlp_headers":
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        headers = dynamic_otlp_headers(input_value["callback_name"], input_value["params"])
        return optional_headers(headers)
    if input_value["function"] == "dynamic_otlp_endpoint":
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        return dynamic_otlp_endpoint(input_value["callback_name"], input_value["params"])
    if input_value["function"] == "project_routing_headers":
        return string_map(
            project_routing_headers(input_value["callback_name"], input_value["auth_metadata"])
        )
    raise ValueError(f"unknown routing function {input_value['function']}")


def run_registry(input_value: Input) -> Json:
    if input_value["function"] == "preset_for":
        return input_value["callback_name"] in PRESET_BY_CALLBACK
    if input_value["function"] == "destination_capable_backends":
        return strings_json(sorted(destination_capable_backends()))
    raise ValueError(f"unknown registry function {input_value['function']}")


def run_destination(input_value: Input) -> Json:
    if input_value["function"] == "destination_for":
        apply_environment(input_value["environment"])
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        destination = destination_for(
            input_value["callback_name"],
            input_value["params"],
            input_value["service_name"],
        )
        if destination is None:
            return None
        return destination_json(destination)
    raise ValueError(f"unknown destination function {input_value['function']}")


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function in _PRESET_FUNCTIONS:
        return run_preset(input_value)
    if function in _BUILDER_FUNCTIONS:
        return run_builder(input_value)
    if function in _ROUTING_FUNCTIONS:
        return run_routing(input_value)
    if function in _REGISTRY_FUNCTIONS:
        return run_registry(input_value)
    return run_destination(input_value)
