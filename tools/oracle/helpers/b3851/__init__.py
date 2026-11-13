import os
from collections.abc import Mapping

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
from pydantic import JsonValue as Json

from .types import BaseInput, Environment, Fields, Input

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


def base_config(fields: BaseInput) -> OpenTelemetryV2Config:
    config = OpenTelemetryV2Config()
    config.exporters = [ExporterSpec.model_validate(item) for item in fields["exporters"]]
    config.mapper_names = list(fields["mapper_names"])
    config.resource_attributes = dict(fields["resource_attributes"])
    return config


def exporter_json(spec: ExporterSpec) -> Fields:
    options: dict[str, Json] | None = None
    if spec.options is not None:
        options = dict(spec.options)
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
        "mapper_names": list(config.mapper_names),
        "resource_attributes": dict(config.resource_attributes),
        "service_name": config.service_name,
        "deployment_environment": config.deployment_environment,
        "capture_message_content": config.capture_message_content,
    }


def destination_json(destination: OtelDestination) -> Fields:
    result: Fields = {}
    result["endpoint"] = destination.endpoint
    result["headers"] = dict(destination.headers)
    result["resource_attributes"] = dict(destination.resource_attributes)
    result["callback_name"] = destination.callback_name
    result["protocol"] = destination.protocol
    result["span_scope"] = destination.span_scope
    return result


def optional_headers(headers: Mapping[str, str] | None) -> Json:
    if headers is None:
        return None
    return dict(headers)


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
        return dict(newrelic_dynamic_headers(input_value["params"]))
    if input_value["function"] == "newrelic_dynamic_endpoint":
        return newrelic_dynamic_endpoint(input_value["params"])
    if input_value["function"] == "signoz_dynamic_headers":
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        return dict(signoz_dynamic_headers(input_value["params"]))
    if input_value["function"] == "signoz_dynamic_endpoint":
        litellm.provider_url_destination_allowed_hosts = input_value["allowed_hosts"]
        return signoz_dynamic_endpoint(input_value["params"])
    if input_value["function"] == "langfuse_dynamic_headers":
        return dict(langfuse_dynamic_headers(input_value["params"]))
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
        return dict(
            project_routing_headers(input_value["callback_name"], input_value["auth_metadata"])
        )
    raise ValueError(f"unknown routing function {input_value['function']}")


def run_registry(input_value: Input) -> Json:
    if input_value["function"] == "preset_for":
        return input_value["callback_name"] in PRESET_BY_CALLBACK
    if input_value["function"] == "destination_capable_backends":
        return [*sorted(destination_capable_backends())]
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
