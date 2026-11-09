import datetime as dt
import random
import sys
from collections.abc import Mapping, Sequence
from enum import Enum
from types import ModuleType, SimpleNamespace
from typing import Annotated, Literal, NamedTuple, Required, TypeAlias, cast
from unittest.mock import patch

from pydantic import BaseModel, Field
from typing_extensions import TypedDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Stub:
    def __init__(self, *args: object, **kwargs: object) -> None:
        del args, kwargs


class _Attributes:
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
    RELEASE = "langfuse.release"
    VERSION = "langfuse.version"


class _PromptClient(BaseModel):
    name: str
    version: int
    is_fallback: bool


class _SpanLimits(_Stub):
    UNSET = -1


class _Decision(Enum):
    DROP = 0
    RECORD_ONLY = 1
    RECORD_AND_SAMPLE = 2


class _SamplingResult(NamedTuple):
    decision: _Decision
    attributes: Mapping[str, object] | None
    trace_state: object


class _RandomIdGenerator:
    def generate_trace_id(self) -> int:
        return random.getrandbits(128) or 1

    def generate_span_id(self) -> int:
        return random.getrandbits(64) or 1


def _install(name: str, **members: object) -> None:
    module = ModuleType(name)
    module.__path__ = []
    vars(module).update(members)
    sys.modules[name] = module


def _stubs(*names: str) -> dict[str, object]:
    return {name: type(name, (_Stub,), {}) for name in names}


# the oracle venv installs litellm[proxy], which leaves out the langfuse package and the OTel SDK
_install("langfuse", LangfuseOtelSpanAttributes=_Attributes)
_install("langfuse.api", **_stubs("LangfuseAPI", "Prompt", "Prompt_Chat"))
_install("langfuse.api.core")
_install("langfuse.api.core.api_error", **_stubs("ApiError"))
_install("langfuse.api.core.request_options", RequestOptions=_Stub)
_install(
    "langfuse.model",
    BasePromptClient=_PromptClient,
    PromptClient=_PromptClient,
    ChatPromptClient=_PromptClient,
    TextPromptClient=_PromptClient,
)
_install("opentelemetry.sdk")
_install("opentelemetry.sdk.resources", **_stubs("Resource"))
_install(
    "opentelemetry.sdk.trace",
    SpanLimits=_SpanLimits,
    **_stubs("ReadableSpan", "TracerProvider"),
)
_install(
    "opentelemetry.sdk.trace.export",
    **_stubs("BatchSpanProcessor", "SpanExporter", "SpanExportResult"),
)
_install("opentelemetry.sdk.trace.id_generator", RandomIdGenerator=_RandomIdGenerator)
_install(
    "opentelemetry.sdk.trace.sampling",
    ALWAYS_ON=_Stub(),
    Decision=_Decision,
    Sampler=_Stub,
    SamplingResult=_SamplingResult,
)
_install("opentelemetry.exporter")
_install("opentelemetry.exporter.otlp")
_install("opentelemetry.exporter.otlp.proto")
_install("opentelemetry.exporter.otlp.proto.common")
_install("opentelemetry.exporter.otlp.proto.common.trace_encoder", encode_spans=bytes)

from litellm.integrations.langfuse import langfuse_sdk as sdk  # noqa: E402 - needs the stubs above
from opentelemetry import trace as otel_trace  # noqa: E402 - the OTel API is installed and must not be stubbed
from opentelemetry.context import Context  # noqa: E402 - as above
from opentelemetry.trace import NonRecordingSpan, SpanContext, TraceFlags, TraceState  # noqa: E402 - as above


class _TraceFields(TypedDict, total=False):
    name: object
    user_id: object
    session_id: object
    version: object
    release: object
    tags: object
    metadata: object
    public: bool | None
    input: object
    output: object


class _ObservationFields(TypedDict, total=False):
    observation_type: Required[Literal["generation", "span"]]
    input: object
    output: object
    metadata: object
    level: object
    status_message: object
    version: object
    model: object
    model_parameters: object
    usage_details: object
    cost_details: object
    completion_start_time: dt.datetime | float | None
    prompt: _PromptClient | None


def _hex_text(value: int | None, width: int) -> str | None:
    return None if value is None else format(value, f"0{width}x")


def _attribute_value(value: object) -> Json:
    if isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (list, tuple)):
        return [_attribute_value(item) for item in cast("Sequence[object]", value)]
    raise ValueError(f"unexpected attribute value: {type(value).__name__}")


def _pairs(attributes: Mapping[str, object]) -> list[Json]:
    return [[key, _attribute_value(value)] for key, value in attributes.items()]


def _context_parts(context: Context | None) -> dict[str, Json]:
    span = otel_trace.get_current_span(context).get_span_context()
    return {
        "valid": span.is_valid,
        "trace_id": _hex_text(span.trace_id if span.is_valid else None, 32),
        "span_id": _hex_text(span.span_id if span.is_valid else None, 16),
        "is_remote": span.is_remote,
        "sampled": span.trace_flags.sampled,
    }


class _Timed(BaseModel):
    seconds: float | None = None
    datetime: dt.datetime | None = None

    @property
    def moment(self) -> dt.datetime | float | None:
        return self.datetime if self.datetime is not None else self.seconds


class _ResolveTraceId(BaseModel):
    function: Literal["resolve_trace_id"]
    input: str | None = None

    def run(self) -> Json:
        return sdk.resolve_trace_id(self.input)


class _ResolveObservationId(BaseModel):
    function: Literal["resolve_observation_id"]
    input: str | None = None

    def run(self) -> Json:
        return sdk.resolve_observation_id(self.input)


class _ToUnixNanos(_Timed):
    function: Literal["to_unix_nanos"]

    def run(self) -> Json:
        return sdk.to_unix_nanos(self.moment)


class _TraceAttributes(BaseModel):
    function: Literal["trace_attributes"]
    fields: _TraceFields

    def run(self) -> Json:
        return _pairs(sdk.trace_attributes(**self.fields))


class _ObservationAttributes(BaseModel):
    function: Literal["observation_attributes"]
    fields: _ObservationFields

    def run(self) -> Json:
        return _pairs(sdk.observation_attributes(**self.fields))


class _Sampler(BaseModel):
    function: Literal["sampler"]
    rate: float
    trace_id: str
    attributes: list[tuple[str, str]]

    def run(self) -> Json:
        sampler = sdk.TraceIdHashSampler(rate=self.rate)
        result = cast(
            "_SamplingResult",
            sampler.should_sample(  # pyright: ignore[reportUnknownMemberType]
                None, int(self.trace_id, 16), "span", attributes=dict(self.attributes)
            ),
        )
        kept = result.attributes
        return {
            "decision": result.decision.name.lower(),
            "attributes": None if kept is None else _pairs(kept),
        }


class _ParentContext(BaseModel):
    function: Literal["parent_context"]
    trace_id: str
    parent_observation_id: str | None
    existing_trace: bool
    fallback_span_id: str

    def run(self) -> Json:
        # the fallback parent id is random in LiteLLM; a fixed one keeps both sides comparable
        fixed = SimpleNamespace(generate_span_id=lambda: int(self.fallback_span_id, 16))
        with patch.object(sdk, "RandomIdGenerator", lambda: fixed):
            context = sdk._parent_context(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                trace_id=self.trace_id,
                parent_observation_id=self.parent_observation_id,
                existing_trace=self.existing_trace,
            )
        return _context_parts(context)


class _RecordingTracer:
    def __init__(self, span_id: int) -> None:
        self.span_id = span_id
        self.recorded: dict[str, Json] = {}

    def start_span(
        self,
        *,
        name: str,
        context: Context,
        start_time: int | None,
        attributes: Mapping[str, object],
    ) -> NonRecordingSpan:
        requested_trace = sdk._requested_trace_id.get()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        requested_span = sdk._requested_span_id.get()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        self.recorded = {
            "requested_trace_id": _hex_text(requested_trace, 32),
            "requested_span_id": _hex_text(requested_span, 16),
            "name": name,
            "start_time": start_time,
            "attributes": _pairs(attributes),
            "parent": _context_parts(context),
        }
        return NonRecordingSpan(SpanContext(0, self.span_id, is_remote=False))


class _StartChildSpan(_Timed):
    function: Literal["start_child_span"]
    parent_trace_id: str
    parent_span_id: str
    parent_public: bool | None
    name: str
    attributes: list[tuple[str, str]]
    child_span_id: str

    def run(self) -> Json:
        tracer = _RecordingTracer(int(self.child_span_id, 16))
        parent_context = SpanContext(
            trace_id=int(self.parent_trace_id, 16),
            span_id=int(self.parent_span_id, 16),
            is_remote=False,
            trace_flags=TraceFlags(TraceFlags.SAMPLED),
            trace_state=TraceState(),
        )
        observation = sdk.start_child_span(
            tracing=cast("sdk.LangfuseTracing", SimpleNamespace(tracer=tracer)),
            parent=sdk.LangfuseObservation(
                span=NonRecordingSpan(parent_context), public=self.parent_public
            ),
            name=self.name,
            start_time=self.moment,
            attributes=dict(self.attributes),
        )
        return {**tracer.recorded, "observation_id": observation.id, "public": observation.public}


_Step: TypeAlias = Annotated[
    _ResolveTraceId
    | _ResolveObservationId
    | _ToUnixNanos
    | _TraceAttributes
    | _ObservationAttributes
    | _Sampler
    | _ParentContext
    | _StartChildSpan,
    Field(discriminator="function"),
]


class _Input(BaseModel):
    steps: list[_Step]


def run(input_value: dict[str, Json]) -> Json:
    return [step.run() for step in _Input.model_validate(input_value).steps]
