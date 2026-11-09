import random
import sys
from collections.abc import Callable, Mapping, Sequence
from contextvars import ContextVar
from datetime import datetime
from enum import Enum
from functools import partial
from types import ModuleType
from typing import Protocol, TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Stub:
    def __init__(self, *args: object, **kwargs: object) -> None:
        del args, kwargs


class _StubMeta(type):
    def __getattr__(cls, name: str) -> object:
        if name.startswith("__"):
            raise AttributeError(name)
        return _stub_class(name)


def _stub_class(name: str) -> type[_Stub]:
    return _StubMeta(name, (_Stub,), {})


class _StubModule(ModuleType):
    def __getattr__(self, name: str) -> object:
        if name.startswith("__"):
            raise AttributeError(name)
        return _stub_class(name)

    def attach(self, name: str, value: object) -> None:
        super().__setattr__(name, value)


def _module(name: str) -> _StubModule:
    module = _StubModule(name)
    module.__path__ = []
    sys.modules[name] = module
    return module


class _LangfuseOtelSpanAttributes:
    TRACE_NAME = "langfuse.trace.name"
    TRACE_USER_ID = "user.id"
    TRACE_SESSION_ID = "session.id"
    TRACE_TAGS = "langfuse.trace.tags"
    TRACE_PUBLIC = "langfuse.trace.public"
    TRACE_METADATA = "langfuse.trace.metadata"
    TRACE_INPUT = "langfuse.trace.input"
    TRACE_OUTPUT = "langfuse.trace.output"
    OBSERVATION_TYPE = "langfuse.observation.type"
    OBSERVATION_METADATA = "langfuse.observation.metadata"
    OBSERVATION_LEVEL = "langfuse.observation.level"
    OBSERVATION_STATUS_MESSAGE = "langfuse.observation.status_message"
    OBSERVATION_INPUT = "langfuse.observation.input"
    OBSERVATION_OUTPUT = "langfuse.observation.output"
    OBSERVATION_COMPLETION_START_TIME = "langfuse.observation.completion_start_time"
    OBSERVATION_MODEL = "langfuse.observation.model.name"
    OBSERVATION_MODEL_PARAMETERS = "langfuse.observation.model.parameters"
    OBSERVATION_USAGE_DETAILS = "langfuse.observation.usage_details"
    OBSERVATION_COST_DETAILS = "langfuse.observation.cost_details"
    OBSERVATION_PROMPT_NAME = "langfuse.observation.prompt.name"
    OBSERVATION_PROMPT_VERSION = "langfuse.observation.prompt.version"
    ENVIRONMENT = "langfuse.environment"
    RELEASE = "langfuse.release"
    VERSION = "langfuse.version"


class _PromptClient:
    def __init__(self, name: str, version: int | None, *, is_fallback: bool) -> None:
        self.name = name
        self.version = version
        self.is_fallback = is_fallback


class _RequestOptions:
    def __init__(self, **kwargs: object) -> None:
        self.max_retries = kwargs.get("max_retries")


class _SpanLimits:
    UNSET = -1

    def __init__(self, **kwargs: object) -> None:
        self.options = kwargs


class _Sampler:
    def should_sample(self, *args: object, **kwargs: object) -> object:
        raise NotImplementedError

    def get_description(self) -> str:
        return type(self).__name__


class _Decision(Enum):
    DROP = 0
    RECORD_ONLY = 1
    RECORD_AND_SAMPLE = 2


class _SamplingResult:
    def __init__(self, decision: _Decision, attributes: object, trace_state: object) -> None:
        self.decision = decision
        self.attributes = attributes
        self.trace_state = trace_state


class _BaseIdGenerator:
    def generate_trace_id(self) -> int:
        return random.getrandbits(128) or 1

    def generate_span_id(self) -> int:
        return random.getrandbits(64) or 1


class _FakeSpanContext:
    def __init__(
        self,
        trace_id: int = 0,
        span_id: int = 0,
        *,
        is_remote: bool = False,
        sampled: bool = True,
    ) -> None:
        self.trace_id = trace_id
        self.span_id = span_id
        self.is_remote = is_remote
        self.trace_flags = _trace_flags(sampled=sampled)


class _FakeSpan:
    def __init__(self, span_id: int, trace_id: int = 0) -> None:
        self._context = _FakeSpanContext(trace_id=trace_id, span_id=span_id)
        self.end_time: int | None = None

    def get_span_context(self) -> _FakeSpanContext:
        return self._context

    def end(self, end_time: int | None = None) -> None:
        self.end_time = end_time


class _FixedIdGenerator(_BaseIdGenerator):
    def __init__(self, trace_id: int, span_id: int) -> None:
        self.trace_id = trace_id
        self.span_id = span_id

    def generate_trace_id(self) -> int:
        return self.trace_id

    def generate_span_id(self) -> int:
        return self.span_id


def _encode_spans(_spans: object) -> bytes:
    return b""


def _install_stubs() -> None:
    _module("langfuse").attach("LangfuseOtelSpanAttributes", _LangfuseOtelSpanAttributes)
    api = _module("langfuse.api")
    api.attach("LangfuseAPI", _stub_class("LangfuseAPI"))
    api.attach("Prompt", _stub_class("Prompt"))
    api.attach("Prompt_Chat", _stub_class("Prompt_Chat"))
    _module("langfuse.api.core")
    _module("langfuse.api.core.api_error").attach("ApiError", _stub_class("ApiError"))
    _module("langfuse.api.core.request_options").attach("RequestOptions", _RequestOptions)
    model = _module("langfuse.model")
    model.attach("BasePromptClient", _PromptClient)
    model.attach("PromptClient", _PromptClient)
    model.attach("ChatPromptClient", _PromptClient)
    model.attach("TextPromptClient", _PromptClient)
    _module("opentelemetry.sdk")
    _module("opentelemetry.sdk.resources").attach("Resource", _stub_class("Resource"))
    trace = _module("opentelemetry.sdk.trace")
    trace.attach("ReadableSpan", _stub_class("ReadableSpan"))
    trace.attach("SpanLimits", _SpanLimits)
    trace.attach("TracerProvider", _stub_class("TracerProvider"))
    export = _module("opentelemetry.sdk.trace.export")
    export.attach("BatchSpanProcessor", _stub_class("BatchSpanProcessor"))
    export.attach("SpanExporter", _stub_class("SpanExporter"))
    export.attach("SpanExportResult", _stub_class("SpanExportResult"))
    _module("opentelemetry.sdk.trace.id_generator").attach("RandomIdGenerator", _BaseIdGenerator)
    sampling = _module("opentelemetry.sdk.trace.sampling")
    sampling.attach("ALWAYS_ON", _Sampler())
    sampling.attach("Decision", _Decision)
    sampling.attach("Sampler", _Sampler)
    sampling.attach("SamplingResult", _SamplingResult)
    _module("opentelemetry.exporter")
    _module("opentelemetry.exporter.otlp")
    _module("opentelemetry.exporter.otlp.proto")
    _module("opentelemetry.exporter.otlp.proto.common")
    encoder = _module("opentelemetry.exporter.otlp.proto.common.trace_encoder")
    encoder.attach("encode_spans", _encode_spans)


# the oracle venv installs litellm[proxy], which leaves out the langfuse package and the OTel SDK
_install_stubs()

from opentelemetry import trace as otel_trace  # noqa: E402 - the OTel API is installed and must not be stubbed
from opentelemetry.context import Context  # noqa: E402 - as above
from opentelemetry.trace import NonRecordingSpan, SpanContext, TraceFlags, TraceState  # noqa: E402 - as above


def _trace_flags(*, sampled: bool) -> TraceFlags:
    return TraceFlags(TraceFlags.SAMPLED if sampled else 0)


class _Observation(Protocol):
    public: bool | None
    span: object

    def end(self, end_time: datetime | float | None = None) -> None: ...

    @property
    def id(self) -> str: ...


class _SamplerResult(Protocol):
    decision: _Decision
    attributes: object
    trace_state: object


class _TraceSampler(Protocol):
    def should_sample(
        self,
        parent_context: Context | None,
        trace_id: int,
        name: str,
        *,
        attributes: Mapping[str, object],
    ) -> _SamplerResult: ...


class _RequestedGenerator(Protocol):
    def generate_trace_id(self) -> int: ...

    def generate_span_id(self) -> int: ...


class _Sdk(Protocol):
    RandomIdGenerator: Callable[[], _BaseIdGenerator]
    LangfuseObservation: Callable[..., _Observation]
    TraceIdHashSampler: Callable[..., _TraceSampler]
    _RequestedIdGenerator: Callable[[], _RequestedGenerator]
    _requested_trace_id: ContextVar[int | None]
    _requested_span_id: ContextVar[int | None]

    def resolve_trace_id(self, trace_id: str | None) -> str: ...

    def resolve_observation_id(self, observation_id: str | None) -> str | None: ...

    def to_unix_nanos(self, value: datetime | float | None) -> int | None: ...

    def trace_attributes(  # noqa: PLR0913 - mirrors LiteLLM's keyword-only signature
        self,
        *,
        name: object = None,
        user_id: object = None,
        session_id: object = None,
        version: object = None,
        release: object = None,
        tags: object = None,
        metadata: object = None,
        public: bool | None = None,
        input: object = None,  # noqa: A002 - LiteLLM names the argument input
        output: object = None,
    ) -> Mapping[str, object]: ...

    def observation_attributes(  # noqa: PLR0913 - mirrors LiteLLM's keyword-only signature
        self,
        *,
        observation_type: str,
        input: object = None,  # noqa: A002 - LiteLLM names the argument input
        output: object = None,
        metadata: object = None,
        level: object = None,
        status_message: object = None,
        version: object = None,
        model: object = None,
        model_parameters: object = None,
        usage_details: object = None,
        cost_details: object = None,
        completion_start_time: datetime | float | None = None,
        prompt: object = None,
    ) -> Mapping[str, object]: ...

    def start_generation(  # noqa: PLR0913 - mirrors LiteLLM's keyword-only signature
        self,
        *,
        tracing: object,
        trace_id: str,
        parent_observation_id: str | None,
        existing_trace: bool,
        observation_id: str | None,
        name: str,
        start_time: datetime | float | None,
        public: bool | None,
        attributes: Mapping[str, object],
    ) -> _Observation: ...

    def start_child_span(
        self,
        *,
        tracing: object,
        parent: _Observation,
        name: str,
        start_time: datetime | float | None,
        attributes: Mapping[str, object],
    ) -> _Observation: ...

    def _parent_context(
        self, *, trace_id: str, parent_observation_id: str | None, existing_trace: bool
    ) -> Context: ...


def _sdk() -> _Sdk:
    from litellm.integrations.langfuse import langfuse_sdk  # noqa: PLC0415 - the langfuse package is stubbed at import time

    return cast(_Sdk, langfuse_sdk)


class _Tracing:
    def __init__(self, tracer: "_FakeTracer") -> None:
        self.tracer = tracer


class _FakeTracer:
    def __init__(self, span_id: int) -> None:
        self.span_id = span_id
        self.recorded: dict[str, Json] = {}

    def start_span(
        self,
        *,
        name: str,
        context: object,
        start_time: int | None,
        attributes: Mapping[str, object],
    ) -> _FakeSpan:
        sdk = _sdk()
        parent = _span_context(context)
        requested_trace = sdk._requested_trace_id.get()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        requested_span = sdk._requested_span_id.get()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        self.recorded = {
            "requested_trace_id": _hex_text(requested_trace, 32),
            "requested_span_id": _hex_text(requested_span, 16),
            "name": name,
            "start_time": start_time,
            "attributes": _pairs(attributes),
            "parent_valid": parent.is_valid,
            "parent_trace_id": _hex_text(parent.trace_id if parent.is_valid else None, 32),
            "parent_span_id": _hex_text(parent.span_id if parent.is_valid else None, 16),
            "parent_is_remote": parent.is_remote,
        }
        return _FakeSpan(self.span_id)


def _hex_text(value: int | None, width: int) -> str | None:
    return None if value is None else format(value, f"0{width}x")


def _span_context(context: object) -> SpanContext:
    span = otel_trace.get_current_span(cast("Context | None", context))
    return span.get_span_context()


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast("dict[str, Json]", value)


def _optional_text(value: Json, name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string or null")
    return value


def _optional_bool(value: Json, name: str) -> bool | None:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a bool or null")
    return value


def _required_text(value: Json, name: str) -> str:
    text = _optional_text(value, name)
    if text is None:
        raise ValueError(f"{name} must be a string")
    return text


def _optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise ValueError(f"{name} must be a number or null")
    return float(value)


def _pairs(attributes: Mapping[str, object]) -> list[Json]:
    pairs: list[Json] = []
    for key, value in attributes.items():
        pairs.append(cast("Json", [key, _attribute_value(value)]))
    return pairs


def _attribute_value(value: object) -> Json:
    if isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (list, tuple)):
        items = cast("Sequence[object]", value)
        return [_attribute_value(item) for item in items]
    raise ValueError(f"unexpected attribute value: {type(value).__name__}")


def _pairs_from_input(value: Json, name: str) -> dict[str, Json]:
    entries = cast("list[Json]", value if isinstance(value, list) else [])
    result: dict[str, Json] = {}
    for entry in entries:
        pair = cast("list[Json]", entry if isinstance(entry, list) else [])
        match pair:
            case [key_value, item]:
                result[_required_text(key_value, f"{name} key")] = item
            case _:
                raise ValueError(f"{name} entries must be [key, value] pairs")
    return result


def _parse_timestamp(value: Json) -> datetime | float | None:
    if value is None:
        return None
    if isinstance(value, str):
        return datetime.fromisoformat(value)
    return _optional_number(value, "timestamp")


def _timestamp(step: dict[str, Json]) -> datetime | float | None:
    if step.get("datetime") is not None:
        return _parse_timestamp(step.get("datetime"))
    return _parse_timestamp(step.get("seconds"))


def _resolve_trace_id(step: dict[str, Json]) -> Json:
    return _sdk().resolve_trace_id(_optional_text(step.get("input"), "input"))


def _resolve_observation_id(step: dict[str, Json]) -> Json:
    return _sdk().resolve_observation_id(_optional_text(step.get("input"), "input"))


def _to_unix_nanos(step: dict[str, Json]) -> Json:
    return _sdk().to_unix_nanos(_timestamp(step))


def _trace_attributes(step: dict[str, Json]) -> Json:
    fields = _object(step.get("fields", {}), "fields")
    return _pairs(
        _sdk().trace_attributes(
            name=fields.get("name"),
            user_id=fields.get("user_id"),
            session_id=fields.get("session_id"),
            version=fields.get("version"),
            release=fields.get("release"),
            tags=fields.get("tags"),
            metadata=fields.get("metadata"),
            public=_optional_bool(fields.get("public"), "public"),
            input=fields.get("input"),
            output=fields.get("output"),
        )
    )


def _prompt_from(fields: dict[str, Json]) -> _PromptClient:
    version = fields.get("version")
    if version is not None and not isinstance(version, int):
        raise ValueError("prompt.version must be an int or null")
    return _PromptClient(
        name=_required_text(fields.get("name"), "prompt.name"),
        version=version,
        is_fallback=_optional_bool(fields.get("is_fallback"), "prompt.is_fallback") is True,
    )


def _observation_attributes(step: dict[str, Json]) -> Json:
    fields = _object(step.get("fields", {}), "fields")
    prompt_value = fields.get("prompt")
    prompt = None if prompt_value is None else _prompt_from(_object(prompt_value, "prompt"))
    return _pairs(
        _sdk().observation_attributes(
            observation_type=_required_text(fields.get("observation_type"), "observation_type"),
            input=fields.get("input"),
            output=fields.get("output"),
            metadata=fields.get("metadata"),
            level=fields.get("level"),
            status_message=fields.get("status_message"),
            version=fields.get("version"),
            model=fields.get("model"),
            model_parameters=fields.get("model_parameters"),
            usage_details=fields.get("usage_details"),
            cost_details=fields.get("cost_details"),
            completion_start_time=_parse_timestamp(fields.get("completion_start_time")),
            prompt=prompt,
        )
    )


def _requested_ids(step: dict[str, Json]) -> Json:
    sdk = _sdk()
    trace_id = int(_required_text(step.get("trace_id"), "trace_id"), 16)
    span_id = int(_required_text(step.get("span_id"), "span_id"), 16)
    generator = sdk._RequestedIdGenerator()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    trace_token = sdk._requested_trace_id.set(trace_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    span_token = sdk._requested_span_id.set(span_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        requested_trace = generator.generate_trace_id()
        requested_span = generator.generate_span_id()
    finally:
        sdk._requested_span_id.reset(span_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        sdk._requested_trace_id.reset(trace_token)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    after_trace = generator.generate_trace_id()
    after_span = generator.generate_span_id()
    return {
        "trace_id": format(requested_trace, "032x"),
        "span_id": format(requested_span, "016x"),
        "after_trace_id_length": len(format(after_trace, "032x")),
        "after_span_id_length": len(format(after_span, "016x")),
        "after_trace_id_nonzero": after_trace != 0,
        "after_span_id_nonzero": after_span != 0,
    }


def _sampler(step: dict[str, Json]) -> Json:
    rate = _optional_number(step.get("rate"), "rate")
    if rate is None:
        raise ValueError("rate must be a number")
    trace_id = int(_required_text(step.get("trace_id"), "trace_id"), 16)
    attributes = _pairs_from_input(step.get("attributes", {}), "attributes")
    result = _sdk().TraceIdHashSampler(rate=rate).should_sample(
        None, trace_id, "span", attributes=attributes
    )
    kept = cast("Mapping[str, object] | None", result.attributes)
    return {
        "decision": result.decision.name.lower(),
        "attributes": None if kept is None else _pairs(kept),
    }


def _parent_context(step: dict[str, Json]) -> Json:
    sdk = _sdk()
    parent_id = _optional_text(step.get("parent_observation_id"), "parent_observation_id")
    trace_id = _required_text(step.get("trace_id"), "trace_id")
    fallback = int(_required_text(step.get("fallback_span_id"), "fallback_span_id"), 16)
    # the fallback parent id is random in LiteLLM; a fixed generator keeps both sides comparable
    sdk.RandomIdGenerator = partial(_FixedIdGenerator, int(trace_id, 16), fallback)
    existing = _optional_bool(step.get("existing_trace"), "existing_trace")
    context = sdk._parent_context(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        trace_id=trace_id, parent_observation_id=parent_id, existing_trace=existing is True
    )
    span = _span_context(context)
    return {
        "valid": span.is_valid,
        "trace_id": _hex_text(span.trace_id if span.is_valid else None, 32),
        "span_id": _hex_text(span.span_id if span.is_valid else None, 16),
        "is_remote": span.is_remote,
        "sampled": span.trace_flags.sampled,
    }


def _observation(step: dict[str, Json]) -> Json:
    span_id = int(_required_text(step.get("span_id"), "span_id"), 16)
    observation = _sdk().LangfuseObservation(
        span=_FakeSpan(span_id),
        public=_optional_bool(step.get("public"), "public"),
    )
    end_time = _timestamp(step)
    if end_time is not None:
        observation.end(end_time)
    span = cast("_FakeSpan", observation.span)
    return {"id": observation.id, "public": observation.public, "end_time": span.end_time}


def _generation(step: dict[str, Json]) -> Json:
    sdk = _sdk()
    tracer = _FakeTracer(int(_required_text(step.get("span_id"), "span_id"), 16))
    observation = sdk.start_generation(
        tracing=_Tracing(tracer),
        trace_id=_required_text(step.get("trace_id"), "trace_id"),
        parent_observation_id=_optional_text(
            step.get("parent_observation_id"), "parent_observation_id"
        ),
        existing_trace=_optional_bool(step.get("existing_trace"), "existing_trace") is True,
        observation_id=_optional_text(step.get("observation_id"), "observation_id"),
        name=_required_text(step.get("name"), "name"),
        start_time=_timestamp(step),
        public=_optional_bool(step.get("public"), "public"),
        attributes=_pairs_from_input(step.get("attributes", {}), "attributes"),
    )
    return {**tracer.recorded, "observation_id": observation.id, "public": observation.public}


def _child_span(step: dict[str, Json]) -> Json:
    sdk = _sdk()
    tracer = _FakeTracer(int(_required_text(step.get("child_span_id"), "child_span_id"), 16))
    parent_context = SpanContext(
        trace_id=int(_required_text(step.get("parent_trace_id"), "parent_trace_id"), 16),
        span_id=int(_required_text(step.get("parent_span_id"), "parent_span_id"), 16),
        is_remote=False,
        trace_flags=TraceFlags(TraceFlags.SAMPLED),
        trace_state=TraceState(),
    )
    parent = sdk.LangfuseObservation(
        span=NonRecordingSpan(parent_context),
        public=_optional_bool(step.get("parent_public"), "parent_public"),
    )
    observation = sdk.start_child_span(
        tracing=_Tracing(tracer),
        parent=parent,
        name=_required_text(step.get("name"), "name"),
        start_time=_timestamp(step),
        attributes=_pairs_from_input(step.get("attributes", {}), "attributes"),
    )
    return {**tracer.recorded, "observation_id": observation.id, "public": observation.public}


_Handlers: TypeAlias = dict[str, Callable[[dict[str, Json]], Json]]
_STEPS: _Handlers = {
    "resolve_trace_id": _resolve_trace_id,
    "resolve_observation_id": _resolve_observation_id,
    "to_unix_nanos": _to_unix_nanos,
    "trace_attributes": _trace_attributes,
    "observation_attributes": _observation_attributes,
    "requested_ids": _requested_ids,
    "sampler": _sampler,
    "parent_context": _parent_context,
    "observation": _observation,
    "start_generation": _generation,
    "start_child_span": _child_span,
}


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    results: list[Json] = []
    for step in steps:
        fields = _object(step, "step")
        function = _required_text(fields.get("function"), "function")
        handler = _STEPS.get(function)
        if handler is None:
            raise ValueError(f"unknown function: {function}")
        results.append(handler(fields))
    return results
