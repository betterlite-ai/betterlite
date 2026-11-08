import json
import sys
from datetime import datetime
from types import ModuleType
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.opentelemetry import (
    OpenTelemetry,
    OpenTelemetryConfig,
    _normalize_team_metadata_keys,  # pyright: ignore[reportPrivateUsage]
)
from litellm.integrations.opentelemetry_utils.gen_ai_semconv import OTELSemconvCategory

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Attributes: TypeAlias = list[list[Json]]


class InMemorySpanExporter:
    pass


def _install_in_memory_exporter() -> None:
    names = (
        "opentelemetry.sdk",
        "opentelemetry.sdk.trace",
        "opentelemetry.sdk.trace.export",
        "opentelemetry.sdk.trace.export.in_memory_span_exporter",
    )
    for name in names:
        module = ModuleType(name)
        module.__path__ = []
        sys.modules[name] = module
    exporter_module = vars(sys.modules[names[-1]])
    cast("dict[str, object]", exporter_module)["InMemorySpanExporter"] = InMemorySpanExporter


_install_in_memory_exporter()


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: Attributes = []
        self.events: list[dict[str, Json]] = []

    def set_attribute(self, key: str, value: Json) -> None:
        if key == "litellm.team.metadata" and isinstance(value, str):
            self.attributes.append([key, cast(Json, json.loads(value))])
            return
        self.attributes.append([key, value])

    def record_exception(self, exception: BaseException) -> None:
        if exception.__traceback__ is not None:
            raise ValueError("the helper builds exceptions without a traceback")
        attributes: Attributes = [
            ["exception.type", type(exception).__qualname__],
            ["exception.message", str(exception)],
        ]
        self.events.append({"name": "exception", "attributes": cast(Json, attributes)})


class OtherLogger(OpenTelemetry):
    pass


_LOGGER_CLASSES: dict[str, type[OpenTelemetry]] = {
    "OpenTelemetry": OpenTelemetry,
    "OtherLogger": OtherLogger,
}


def run(input_value: dict[str, Json]) -> Json:  # noqa: C901, PLR0911, PLR0912 # one branch per helper function
    function = input_value.get("function")
    if function == "config_from_env":
        return config_value(OpenTelemetryConfig.from_env())
    if function == "config_normalize":
        return config_value(config_from(cast("dict[str, Json]", input_value.get("config"))))
    if function == "metadata_keys":
        values = cast("list[Json]", input_value.get("values", []))
        keys: list[Json] = []
        for value in values:
            normalized = _normalize_team_metadata_keys(cast("str | list[object] | None", value))
            keys.append(cast(Json, normalized))
        return keys
    if function == "capture_sample":
        return capture_sample(input_value)
    if function == "capture_resolve":
        return capture_resolve(input_value)
    if function == "emit_once":
        return emit_once(input_value)
    if function == "span_time":
        return span_time(input_value)
    if function == "team":
        return team_attributes(input_value)
    if function == "team_from_payload":
        return team_from_payload(input_value)
    if function == "identity":
        return identity_attributes(input_value)
    if function == "service_tier":
        return service_tier_attributes(input_value)
    if function == "exception_stamp":
        return exception_stamp(input_value)
    if function == "semconv_request":
        return semconv_request(input_value)
    if function == "semconv_cache_and_operation":
        return semconv_cache_and_operation(input_value)
    if function == "semconv_gate":
        config = config_from(cast("dict[str, Json]", input_value.get("config")))
        logger = new_logger(config)
        return logger._gen_ai_semconv_latest_experimental  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    raise ValueError(f"unknown {litellm.__name__} helper function {function}")


def scope_value(value: Json) -> object:
    if value == "Null":
        return None
    tag, inner = next(iter(cast("dict[str, Json]", value).items()))
    if tag in ("Bool", "Int", "Float", "Text"):
        return inner
    if tag == "List":
        return [scope_value(item) for item in cast("list[Json]", inner)]
    if tag == "Set":
        return {scope_value(item) for item in cast("list[Json]", inner)}
    if tag == "Map":
        return {key: scope_value(item) for key, item in cast("dict[str, Json]", inner).items()}
    raise ValueError(f"unknown scope tag {tag}")


def config_from(fields: dict[str, Json]) -> OpenTelemetryConfig:
    exporter = fields.get("exporter")
    exporter_value: object = "console" if exporter is None else exporter
    if exporter_value == "in_memory":
        exporter_value = InMemorySpanExporter()
    signals = cast("dict[str, Json]", fields.get("signals", {}))
    return OpenTelemetryConfig(
        exporter=cast("str", exporter_value),
        endpoint=cast("str | None", fields.get("endpoint")),
        headers=cast("str | None", fields.get("headers")),
        enable_metrics=cast(bool, signals.get("enable_metrics", False)),
        enable_events=cast(bool, signals.get("enable_events", False)),
        service_name=cast("str | None", fields.get("service_name")),
        deployment_environment=cast("str | None", fields.get("deployment_environment")),
        model_id=cast("str | None", fields.get("model_id")),
        ignore_context_propagation=cast("bool | None", fields.get("ignore_context_propagation")),
        skip_set_global=cast(bool, fields.get("skip_set_global", False)),
        capture_message_content=cast("str | None", fields.get("capture_message_content")),
        semconv_stability_opt_in={
            OTELSemconvCategory(cast(str, item))
            for item in cast("list[Json]", fields.get("semconv_stability_opt_in", []))
        },
        baggage_team_metadata_keys=cast("list[str]", fields.get("baggage_team_metadata_keys", [])),
        baggage_metadata_keys=cast("list[str]", fields.get("baggage_metadata_keys", [])),
    )


def config_value(config: OpenTelemetryConfig) -> Json:
    exporter = config.exporter if isinstance(config.exporter, str) else "in_memory"  # pyright: ignore[reportUnknownMemberType]
    return cast(
        Json,
        {
            "exporter": exporter,
            "endpoint": config.endpoint,
            "headers": config.headers,
            "enable_metrics": config.enable_metrics,
            "enable_events": config.enable_events,
            "service_name": config.service_name,
            "deployment_environment": config.deployment_environment,
            "model_id": config.model_id,
            "ignore_context_propagation": bool(config.ignore_context_propagation),
            "skip_set_global": config.skip_set_global,
            "capture_message_content": config.capture_message_content,
            "semconv_stability_opt_in": sorted(
                category.value for category in config.semconv_stability_opt_in
            ),
            "baggage_team_metadata_keys": list(config.baggage_team_metadata_keys),
            "baggage_metadata_keys": list(config.baggage_metadata_keys),
            "attributes": None,
        },
    )


def new_logger(config: OpenTelemetryConfig) -> OpenTelemetry:
    logger = OpenTelemetry.__new__(OpenTelemetry)
    logger.config = config
    return logger


def capture_sample(input_value: dict[str, Json]) -> Json:
    sampled: list[Json] = []
    for config_value_input in cast("list[Json]", input_value.get("configs", [])):
        logger = new_logger(config_from(cast("dict[str, Json]", config_value_input)))
        sampled_value = logger._compute_capture_mode_from_init_state()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        logger._capture_mode_cached = sampled_value  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        resolved: dict[str, Json] = {}
        variants = ((False, "without_message_logging"), (True, "with_message_logging"))
        for message_logging, name in variants:
            logger.message_logging = message_logging
            resolved[name] = logger._resolve_capture_mode()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        sampled.append(resolved)
    return sampled


def capture_resolve(input_value: dict[str, Json]) -> Json:
    logger = new_logger(config_from(cast("dict[str, Json]", input_value.get("config"))))
    sampled = logger._compute_capture_mode_from_init_state()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    logger._capture_mode_cached = sampled  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    logger.message_logging = cast(bool, input_value.get("message_logging"))
    litellm.turn_off_message_logging = cast(bool, input_value.get("turn_off_message_logging"))
    mode = logger._resolve_capture_mode()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {
        "mode": mode,
        "in_span": logger._capture_in_span(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "in_event": logger._capture_in_event(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    }


def emit_once(input_value: dict[str, Json]) -> Json:
    kwargs: dict[str, object] = {"litellm_params": {"metadata": {}}}
    loggers: dict[tuple[str, int], OpenTelemetry] = {}
    emitted: list[Json] = []
    for call in cast("list[Json]", input_value.get("calls", [])):
        fields = cast("dict[str, Json]", call)
        handler = cast("dict[str, Json]", fields.get("handler"))
        class_name = cast(str, handler.get("class_name"))
        instance = cast(int, handler.get("instance"))
        logger_class = _LOGGER_CLASSES[class_name]
        logger = loggers.setdefault((class_name, instance), logger_class.__new__(logger_class))
        scope = [scope_value(item) for item in cast("list[Json]", fields.get("scope", []))]
        emitted.append(logger._emit_once(kwargs, *scope))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return emitted


def span_time(input_value: dict[str, Json]) -> Json:
    logger = new_logger(OpenTelemetryConfig())
    nanoseconds: list[Json] = []
    for time in cast("list[Json]", input_value.get("times", [])):
        fields = cast("dict[str, Json]", time)
        if "seconds" in fields:
            nanoseconds.append(logger._to_ns(cast(datetime, fields["seconds"])))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        else:
            date_time = datetime.fromisoformat(cast(str, fields.get("date_time")))
            nanoseconds.append(logger._to_ns(date_time))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    names: list[Json] = []
    for name in cast("list[Json]", input_value.get("names", [])):
        fields = cast("dict[str, Json]", name)
        categories: set[OTELSemconvCategory] = (
            {OTELSemconvCategory("gen_ai_latest_experimental")}
            if cast(bool, fields.get("semconv", False))
            else set()
        )
        kwargs: dict[str, object] = {
            "litellm_params": fields.get("litellm_params", {}),
            "call_type": fields.get("call_type"),
            "model": fields.get("model"),
        }
        names.append(
            new_logger(OpenTelemetryConfig(semconv_stability_opt_in=categories))._get_span_name(kwargs)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        )
    return {"ns": nanoseconds, "names": names}


def team_attributes(input_value: dict[str, Json]) -> Json:
    records: list[Json] = []
    for team in cast("list[Json]", input_value.get("teams", [])):
        fields = cast("dict[str, Json]", team)
        span = RecordingSpan()
        new_logger(OpenTelemetryConfig())._set_team_attributes_on_span(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            span,
            cast("str | None", fields.get("team_id")),
            cast("str | None", fields.get("team_alias")),
        )
        records.append({"attributes": cast(Json, span.attributes), "events": []})
    return records


def team_from_payload(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    payload = cast("StandardLoggingPayload", input_value.get("payload"))
    logger = new_logger(OpenTelemetryConfig())
    logger._set_team_attributes_from_kwargs(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        span, {"standard_logging_object": payload}
    )
    return {"attributes": cast(Json, span.attributes), "events": []}


def identity_attributes(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    team_keys = cast("list[str]", input_value.get("baggage_team_metadata_keys", []))
    config = OpenTelemetryConfig(
        baggage_team_metadata_keys=team_keys,
        baggage_metadata_keys=cast("list[str]", input_value.get("baggage_metadata_keys", [])),
    )
    new_logger(config)._set_inference_identity_attributes(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        span,
        cast("StandardLoggingPayload", input_value.get("payload")),
        cast("dict[str, object]", input_value.get("litellm_params", {})),
    )
    return {"attributes": cast(Json, span.attributes), "events": []}


def service_tier_attributes(input_value: dict[str, Json]) -> Json:
    records: list[Json] = []
    for payload_value in cast("list[Json]", input_value.get("payloads", [])):
        span = RecordingSpan()
        payload = cast("StandardLoggingPayload", payload_value)
        logger = new_logger(OpenTelemetryConfig())
        logger._set_service_tier_attributes(span, payload)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        records.append({"attributes": cast(Json, span.attributes), "events": []})
    return records


def exception_stamp(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    record = input_value.get("exception")
    exception: BaseException | None = None
    if record is not None:
        fields = cast("dict[str, Json]", record)
        name = cast(str, fields.get("type_name"))
        class_fields = {"__module__": "builtins"}
        exception_class = cast("type[Exception]", type(name, (Exception,), class_fields))
        exception = exception_class(cast(str, fields.get("message")))
    payload = cast("StandardLoggingPayload", input_value.get("payload"))
    new_logger(OpenTelemetryConfig())._record_exception_on_span(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        span, {"exception": exception, "standard_logging_object": payload}
    )
    return {"attributes": cast(Json, span.attributes), "events": cast(Json, span.events)}


def semconv_request(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    optional_params = cast("dict[str, object]", input_value.get("optional_params"))
    new_logger(OpenTelemetryConfig())._set_semconv_request_attributes(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        span, optional_params
    )
    return {"attributes": cast(Json, span.attributes), "events": []}


def semconv_cache_and_operation(input_value: dict[str, Json]) -> Json:
    cache: list[Json] = []
    for payload_value in cast("list[Json]", input_value.get("payloads", [])):
        span = RecordingSpan()
        payload = cast("StandardLoggingPayload", payload_value)
        new_logger(OpenTelemetryConfig())._set_semconv_cache_token_attributes(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            span, payload
        )
        cache.append({"attributes": cast(Json, span.attributes), "events": []})
    operations: list[Json] = [
        OpenTelemetry._gen_ai_operation_name(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            {"call_type": call_type}
        )
        for call_type in cast("list[Json]", input_value.get("call_types", []))
    ]
    return {"cache": cache, "operations": operations}
