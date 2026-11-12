import os
from typing import Literal, TypeAlias, TypedDict

from litellm.integrations.langfuse.langfuse_otel import LangfuseOtelLogger
from litellm.integrations.opentelemetry import OpenTelemetryConfig
from litellm.types.utils import StandardCallbackDynamicParams
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SpanExporter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Environment: TypeAlias = dict[str, str]


class InputError(ValueError):
    pass


class InitializationInput(TypedDict):
    operation: Literal["initialization"]
    environment: Environment


class HostInput(TypedDict):
    operation: Literal["host"]
    environment: Environment


class GetConfigInput(TypedDict):
    operation: Literal["get_config"]
    environment: Environment


class BuildConfigInput(TypedDict):
    operation: Literal["build_config"]
    environment: Environment
    public_key: str
    secret_key: str
    langfuse_host: str | None


class AuthorizationHeaderInput(TypedDict):
    operation: Literal["authorization_header"]
    public_key: str
    secret_key: str


class BuildHeadersInput(TypedDict):
    operation: Literal["build_headers"]
    auth_header: str


class FormatHeadersInput(TypedDict):
    operation: Literal["format_headers"]
    auth_header: str


class DynamicHeadersInput(TypedDict):
    operation: Literal["dynamic_headers"]
    params: StandardCallbackDynamicParams


class DynamicConfigInput(TypedDict):
    operation: Literal["dynamic_config"]
    environment: Environment
    params: StandardCallbackDynamicParams


Input: TypeAlias = (
    InitializationInput
    | HostInput
    | GetConfigInput
    | BuildConfigInput
    | AuthorizationHeaderInput
    | BuildHeadersInput
    | FormatHeadersInput
    | DynamicHeadersInput
    | DynamicConfigInput
)


def apply_environment(environment: Environment) -> None:
    os.environ.clear()
    os.environ.update(environment)


def config_fields(config: OpenTelemetryConfig) -> Fields:
    match config.exporter:
        case str() as exporter:
            return {
                "exporter": exporter,
                "endpoint": config.endpoint,
                "headers": config.headers,
                "service_name": config.service_name,
                "deployment_environment": config.deployment_environment,
                "model_id": config.model_id,
                "enable_metrics": config.enable_metrics,
                "enable_events": config.enable_events,
            }
        case SpanExporter():
            raise InputError("a span exporter object has no JSON form")


def initialized_config(environment: Environment) -> Fields:
    apply_environment(environment)
    logger = LangfuseOtelLogger(  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L34
        callback_name="langfuse_otel", tracer_provider=TracerProvider()
    )
    return config_fields(logger.config)


def dynamic_logger(environment: Environment) -> LangfuseOtelLogger:
    apply_environment(environment)
    return LangfuseOtelLogger(  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L34
        config=OpenTelemetryConfig(), tracer_provider=TracerProvider()
    )


def run(input_value: Input) -> Json:
    result: Json
    if input_value["operation"] == "initialization":
        result = initialized_config(input_value["environment"])
    elif input_value["operation"] == "host":
        apply_environment(input_value["environment"])
        result = LangfuseOtelLogger._get_langfuse_otel_host()  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L259
    elif input_value["operation"] == "get_config":
        apply_environment(input_value["environment"])
        result = config_fields(LangfuseOtelLogger.get_langfuse_otel_config())  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L290
    elif input_value["operation"] == "build_config":
        apply_environment(input_value["environment"])
        built = LangfuseOtelLogger._build_langfuse_otel_config(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L320
            public_key=input_value["public_key"],
            secret_key=input_value["secret_key"],
            langfuse_host=input_value["langfuse_host"],
        )
        result = config_fields(built)
    elif input_value["operation"] == "authorization_header":
        result = LangfuseOtelLogger._get_langfuse_authorization_header(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L348
            public_key=input_value["public_key"], secret_key=input_value["secret_key"]
        )
    elif input_value["operation"] == "build_headers":
        built_headers = LangfuseOtelLogger._build_langfuse_otel_headers(input_value["auth_header"])  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L357
        result = [[name, value] for name, value in built_headers.items()]
    elif input_value["operation"] == "format_headers":
        headers = LangfuseOtelLogger._build_langfuse_otel_headers(input_value["auth_header"])  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L357
        result = LangfuseOtelLogger._format_otel_headers(headers)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L370
    elif input_value["operation"] == "dynamic_headers":
        logger = dynamic_logger({})
        dynamic_headers: dict[str, str] | None = logger.construct_dynamic_otel_headers(  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/langfuse/langfuse_otel.py#L376
            input_value["params"]
        )
        result = [] if dynamic_headers is None else [[n, v] for n, v in dynamic_headers.items()]
    else:
        logger = dynamic_logger(input_value["environment"])
        dynamic_config = logger.construct_dynamic_otel_config(input_value["params"])
        result = None if dynamic_config is None else config_fields(dynamic_config)
    return result
