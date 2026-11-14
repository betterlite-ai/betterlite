from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from typing import Final, Literal, Protocol

from pydantic import JsonValue

from betterlite.types.router_weights import RouterWeights
from betterlite.types.utils.response import ModelResponse

AgenticSurface = Literal["chat_completions", "responses"]

TRUSTED_CALLBACK_VARS_FIELD: Final = "litellm_trusted_callback_vars"
ADDRESSED_RESPONSE_ID_FIELD: Final = "_litellm_addressed_response_id"

WIRE_NAME: Final = "wire_name"
JsonMap = Mapping[str, JsonValue]


class ForeignObject(Protocol): ...


@dataclass(frozen=True, slots=True, kw_only=True)
class CallState:
    litellm_call_id: str | None = None
    completion_call_id: str | None = None
    model_alias_map: Mapping[str, str] | None = None
    data_residency: str | None = None
    litellm_logging_obj: ForeignObject | None = None
    preset_cache_key: str | None = None
    cache_key: str | None = None
    stream_response: Mapping[str, ModelResponse] | None = None
    context_compaction_state: JsonValue = field(
        default=None, metadata={WIRE_NAME: "_context_compaction_state"}
    )


@dataclass(frozen=True, slots=True, kw_only=True)
class AgenticLoopState:
    depth: int | None = field(default=None, metadata={WIRE_NAME: "_agentic_loop_depth"})
    fingerprints: Sequence[str] | None = field(
        default=None, metadata={WIRE_NAME: "_agentic_loop_fingerprints"}
    )
    api_surface: AgenticSurface | None = field(
        default=None, metadata={WIRE_NAME: "_agentic_loop_api_surface"}
    )
    code_interpreter_active: bool | None = field(
        default=None, metadata={WIRE_NAME: "_code_interpreter_interception_active"}
    )
    code_interpreter_sandbox_key: str | None = field(
        default=None, metadata={WIRE_NAME: "_code_interpreter_interception_sandbox_key"}
    )
    code_interpreter_session_scoped: bool | None = field(  # ast-grep-ignore: no-long-names
        default=None, metadata={WIRE_NAME: "_code_interpreter_interception_session_scoped"}
    )
    code_interpreter_converted_stream: bool | None = field(  # ast-grep-ignore: no-long-names
        default=None, metadata={WIRE_NAME: "_code_interpreter_interception_converted_stream"}
    )
    websearch_emit_native_blocks: bool | None = field(
        default=None, metadata={WIRE_NAME: "_websearch_interception_emit_native_blocks"}
    )
    websearch_converted_stream: bool | None = field(
        default=None, metadata={WIRE_NAME: "_websearch_interception_converted_stream"}
    )
    headroom_converted_stream: bool | None = field(
        default=None, metadata={WIRE_NAME: "_headroom_interception_converted_stream"}
    )


@dataclass(frozen=True, slots=True, kw_only=True)
class RouterState:
    weights: RouterWeights | None = field(default=None, metadata={WIRE_NAME: "_router_weights"})
    fallback_depth: int | None = None
    max_fallbacks: int | None = None
    attempted_targets: JsonValue = None


@dataclass(frozen=True, slots=True, kw_only=True)
class ProxyRequestState:
    proxy_server_request: JsonMap | None = None
    secret_fields: JsonMap | None = None
    trusted_callback_vars: Mapping[str, str] | None = field(
        default=None, metadata={WIRE_NAME: TRUSTED_CALLBACK_VARS_FIELD}
    )
    addressed_response_id: str | None = field(
        default=None, metadata={WIRE_NAME: ADDRESSED_RESPONSE_ID_FIELD}
    )
    strip_stream_usage: bool | None = field(
        default=None, metadata={WIRE_NAME: "_litellm_strip_stream_usage"}
    )
    client_side_timeout: bool | None = None
    model_file_id_mapping: Mapping[str, Mapping[str, str]] | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class EntrypointState:
    acompletion: bool | None = None
    aembedding: bool | None = None
    aimg_generation: bool | None = None
    atext_completion: bool | None = None
    text_completion: bool | None = None
    allm_passthrough_route: bool | None = None
    async_call: bool | None = None


@dataclass(frozen=True, slots=True, kw_only=True)
class InternalState:
    call: CallState
    agentic_loop: AgenticLoopState
    router: RouterState
    proxy: ProxyRequestState
    entrypoint: EntrypointState
