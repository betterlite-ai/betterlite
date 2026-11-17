from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.integrations.otel.logger import _logger_class  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L1021-L1026
from litellm.integrations.otel.model.config import ExporterSpec, OpenTelemetryV2Config
from litellm.litellm_core_utils.litellm_logging import (
    _exports_nowhere,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L5247
    _has_operator_exporter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L5257
    _is_credential_gated,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L5252
    _is_gated,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L5268
    _only_the_gated_exporter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L5264
)
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True
JSON: Final = TypeAdapter(JsonValue)

Function: TypeAlias = Literal[
    "logger_class",
    "is_gated",
    "exports_nowhere",
    "is_credential_gated",
    "has_operator_exporter",
    "only_the_gated_exporter",
]


class ExporterInput(TypedDict):
    kind: NotRequired[str]
    endpoint: NotRequired[str | None]
    traces_endpoint: NotRequired[str | None]
    headers: NotRequired[str | None]
    requires_headers: NotRequired[bool]


class ConfigInput(TypedDict):
    exporters: list[ExporterInput]
    mapper_names: list[str]
    capture_message_content: str


class Input(TypedDict):
    function: Function
    config: ConfigInput


def run(input_value: Input) -> JsonValue:
    fields = input_value["config"]
    config = OpenTelemetryV2Config()
    config.exporters = [ExporterSpec.model_validate(exporter) for exporter in fields["exporters"]]
    config.mapper_names = list(fields["mapper_names"])
    config.capture_message_content = fields["capture_message_content"]
    function = input_value["function"]
    if function == "logger_class":
        return _logger_class(config).__name__
    if function == "is_gated":
        gated: list[bool] = [_is_gated(spec) for spec in config.exporters]
        return JSON.validate_python(gated)
    if function == "exports_nowhere":
        return _exports_nowhere(config)
    if function == "is_credential_gated":
        return _is_credential_gated(config)
    if function == "has_operator_exporter":
        return _has_operator_exporter(config)
    gated_exporters: list[dict[str, str | None | bool]] = [
        {
            "kind": spec.kind,
            "endpoint": spec.endpoint,
            "traces_endpoint": spec.traces_endpoint,
            "headers": spec.headers,
            "requires_headers": spec.requires_headers,
        }
        for spec in _only_the_gated_exporter(config).exporters
    ]
    return JSON.validate_python(gated_exporters)
