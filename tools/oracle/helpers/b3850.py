import sys
from collections.abc import Callable, Mapping
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from types import ModuleType
from typing import TypeAlias, cast

import litellm
from litellm.integrations.arize.arize import ArizeLogger
from litellm.integrations.arize.arize_phoenix import ArizePhoenixLogger
from litellm.integrations.levo.levo import LevoConfig, LevoLogger
from litellm.integrations.otel.model.config import ExporterSpec, OpenTelemetryV2Config
from litellm.integrations.weave.weave_otel import get_weave_otel_config
from litellm.types.integrations.arize import ArizeConfig
from litellm.types.integrations.arize_phoenix import ArizePhoenixConfig
from litellm.types.integrations.weave_otel import WeaveOtelConfig
from litellm.types.utils import StandardCallbackDynamicParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
PresetFunction: TypeAlias = Callable[..., OpenTelemetryV2Config]
DynamicHeaders: TypeAlias = Callable[..., dict[str, str]]
ProjectHeaders: TypeAlias = Callable[..., Mapping[str, str]]

_PRESETS = Path(litellm.__file__).parent / "integrations" / "otel" / "presets"


def _load(name: str, path: Path) -> ModuleType:
    spec = spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load LiteLLM module at {path}")
    module = module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _stub_package(name: str) -> None:
    package = ModuleType(name)
    package.__path__ = []
    sys.modules[name] = package


# the presets package imports opentelemetry-sdk at import, which the pinned proxy extra leaves out
_stub_package("litellm.integrations.otel.presets")
_load("litellm.integrations.otel.presets.utils", _PRESETS / "utils.py")
_ARIZE = _load("litellm.integrations.otel.presets.arize", _PRESETS / "arize.py")
_PHOENIX = _load("litellm.integrations.otel.presets.phoenix", _PRESETS / "phoenix.py")
_LEVO = _load("litellm.integrations.otel.presets.levo", _PRESETS / "levo.py")
_WEAVE = _load("litellm.integrations.otel.presets.weave", _PRESETS / "weave.py")
_LANGTRACE = _load("litellm.integrations.otel.presets.langtrace", _PRESETS / "langtrace.py")

_ARIZE_PRESET = cast(PresetFunction, vars(_ARIZE)["arize_preset"])
_ARIZE_DYNAMIC_HEADERS = cast(DynamicHeaders, vars(_ARIZE)["arize_dynamic_headers"])
_PHOENIX_PRESET = cast(PresetFunction, vars(_PHOENIX)["phoenix_preset"])
_PHOENIX_PROJECT_HEADERS = cast(ProjectHeaders, vars(_PHOENIX)["phoenix_project_headers"])
_LEVO_PRESET = cast(PresetFunction, vars(_LEVO)["levo_preset"])
_WEAVE_PRESET = cast(PresetFunction, vars(_WEAVE)["weave_preset"])
_WEAVE_DYNAMIC_HEADERS = cast(DynamicHeaders, vars(_WEAVE)["weave_dynamic_headers"])
_LANGTRACE_PRESET = cast(PresetFunction, vars(_LANGTRACE)["langtrace_preset"])


def run(input_value: dict[str, Json]) -> Json:  # noqa: C901, PLR0911 # one branch per helper function
    function = input_value.get("function")
    if function == "arize_config":
        return arize_config_value(ArizeLogger.get_arize_config())
    if function == "arize_preset":
        return preset_output(_ARIZE_PRESET(config_overrides=base_config(input_value)))
    if function == "arize_dynamic_headers":
        return dynamic_headers_json(_ARIZE_DYNAMIC_HEADERS, input_value)
    if function == "phoenix_config":
        return phoenix_config_value(ArizePhoenixLogger.get_arize_phoenix_config())
    if function == "phoenix_preset":
        return preset_output(_PHOENIX_PRESET(config_overrides=base_config(input_value)))
    if function == "phoenix_project_headers":
        metadata = string_map(input_value.get("metadata"), "metadata")
        return cast("dict[str, Json]", dict(_PHOENIX_PROJECT_HEADERS(metadata)))
    if function == "levo_config":
        return levo_config_value(LevoLogger.get_levo_config())
    if function == "levo_preset":
        return preset_output(_LEVO_PRESET(config_overrides=base_config(input_value)))
    if function == "weave_config":
        return weave_config_value(get_weave_otel_config())
    if function == "weave_preset":
        return preset_output(
            _WEAVE_PRESET(
                config_overrides=base_config(input_value),
                allow_missing_credentials=input_value.get("allow_missing_credentials") is True,
            )
        )
    if function == "weave_dynamic_headers":
        return dynamic_headers_json(_WEAVE_DYNAMIC_HEADERS, input_value)
    if function == "langtrace_preset":
        return preset_output(_LANGTRACE_PRESET(config_overrides=base_config(input_value)))
    raise ValueError(f"unknown {litellm.__name__} helper function {function}")


def base_config(input_value: dict[str, Json]) -> OpenTelemetryV2Config:
    config = OpenTelemetryV2Config()
    fields = object_value(input_value.get("base"), "base")
    config.exporters = [
        exporter_from(item) for item in list_value(fields.get("exporters", []), "exporters")
    ]
    config.mapper_names = [
        string_value(name, "mapper_names")
        for name in list_value(fields.get("mapper_names", []), "mapper_names")
    ]
    config.resource_attributes = string_map(
        fields.get("resource_attributes", {}), "resource_attributes"
    )
    return config


def exporter_from(value: Json) -> ExporterSpec:
    return ExporterSpec.model_validate(object_value(value, "exporter"))


def dynamic_headers_json(headers: DynamicHeaders, input_value: dict[str, Json]) -> Json:
    params = dynamic_params(input_value.get("params"))
    return cast(dict[str, Json], dict(headers(params)))


def dynamic_params(value: Json | None) -> StandardCallbackDynamicParams:
    return cast(StandardCallbackDynamicParams, string_map(value, "params"))


def preset_output(config: OpenTelemetryV2Config) -> Json:
    exporters: list[Json] = [exporter_json(spec) for spec in config.exporters]
    mapper_names: list[Json] = list(config.mapper_names)
    resource_attributes: dict[str, Json] = cast(
        dict[str, Json], dict(config.resource_attributes)
    )
    return {
        "exporters": exporters,
        "mapper_names": mapper_names,
        "resource_attributes": resource_attributes,
    }


def exporter_json(spec: ExporterSpec) -> dict[str, Json]:
    headers = spec.headers
    options: dict[str, Json] | None = None
    if spec.options is not None:
        options = cast(dict[str, Json], dict(spec.options))
    return {
        "kind": spec.kind,
        "endpoint": spec.endpoint,
        "traces_endpoint": spec.traces_endpoint,
        "headers": None if headers == "" else headers,
        "owner": None if spec.owner is None else spec.owner.value,
        "options": options,
        "use_simple_processor": spec.use_simple_processor,
        "requires_headers": spec.requires_headers,
    }


def arize_config_value(config: ArizeConfig) -> Json:
    return {
        "space_id": config.space_id,
        "space_key": config.space_key,
        "api_key": config.api_key,
        "protocol": config.protocol,
        "endpoint": config.endpoint,
        "project_name": config.project_name,
    }


def phoenix_config_value(config: ArizePhoenixConfig) -> Json:
    return {
        "otlp_auth_headers": config.otlp_auth_headers,
        "protocol": config.protocol,
        "endpoint": config.endpoint,
        "project_name": config.project_name,
    }


def levo_config_value(config: LevoConfig) -> Json:
    return {
        "otlp_auth_headers": config.otlp_auth_headers,
        "protocol": config.protocol,
        "endpoint": config.endpoint,
    }


def weave_config_value(config: WeaveOtelConfig) -> Json:
    return {
        "otlp_auth_headers": config.otlp_auth_headers,
        "endpoint": config.endpoint,
        "project_id": config.project_id,
        "protocol": config.protocol,
    }


def object_value(value: Json | None, field: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be an object")
    return value


def list_value(value: Json | None, field: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{field} must be a list")
    return value


def string_value(value: Json | None, field: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{field} must be a string")
    return value


def string_map(value: Json | None, field: str) -> dict[str, str]:
    fields = object_value(value, field)
    result: dict[str, str] = {}
    for key, item in fields.items():
        if not isinstance(item, str):
            raise ValueError(f"{field}.{key} must be a string")
        result[key] = item
    return result
