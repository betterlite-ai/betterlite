import json
from collections.abc import Callable
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.litellm_core_utils import realtime_errors
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLogging
from litellm.litellm_core_utils.realtime_streaming import BackendClose, backend_close_from
from litellm.litellm_core_utils.realtime_streaming import (
    _decode_json_object as decode_json,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L106-L108
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.realtime.http_transformation import BaseRealtimeHTTPConfig
from litellm.llms.base_llm.realtime.transformation import BaseRealtimeConfig
from litellm.proxy.common_utils.realtime_utils import (
    _realtime_request_body as realtime_request_body,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/realtime_utils.py#L6-L12
)
from litellm.types import realtime as realtime_types
from litellm.types.realtime import RealtimeTranscriptionSessionRequest as TranscriptionRequest
from pydantic import BaseModel
from typing_extensions import override
from websockets.exceptions import ConnectionClosed
from websockets.frames import Close

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class UsageDocument(TypedDict):
    type: Literal["tokens", "duration"]
    input_tokens: int
    output_tokens: int
    total_tokens: int
    text_tokens: int
    audio_tokens: int
    seconds: float


class RecordGroup(TypedDict):
    kind: str
    documents: list[Json]
    usage: list[UsageDocument]


class ErrorDocument(TypedDict):
    message: str
    status_code: int
    headers: dict[str, Json]


class UrlDocument(TypedDict):
    api_base: str | None
    model: str
    api_version: str | None
    ephemeral_key: str


class ReasonDocument(TypedDict):
    message: str
    fallback: str


class FrameDocument(TypedDict):
    code: int
    reason: str


class BackendError(TypedDict):
    received: FrameDocument | None
    sent: FrameDocument | None


class Input(TypedDict):
    function: Literal["records", "hooks", "http_urls", "close_helpers", "cached_body_and_frames"]
    groups: list[RecordGroup]
    complete_urls: list[str | None]
    error: ErrorDocument
    urls: list[UrlDocument]
    models: list[str]
    reason_messages: list[ReasonDocument]
    close_codes: list[int]
    handshake_statuses: list[int]
    backend_closes: list[FrameDocument]
    backend_errors: list[BackendError]
    payloads: list[str]


_EMPTY: dict[str, Json] = {}

_MODELS: dict[str, type[BaseModel]] = {
    "expiry": realtime_types.RealtimeExpiresAfter,
    "session": realtime_types.RealtimeSessionConfig,
    "client_secret_request": realtime_types.RealtimeClientSecretRequest,
    "client_secret_response": realtime_types.RealtimeClientSecretResponse,
    "transcription_response": realtime_types.RealtimeTranscriptionSessionResponse,
}


def _token_usage(document: UsageDocument) -> Json:
    usage = realtime_types.RealtimeInputAudioTranscriptionTokenUsage(
        type="tokens",
        input_tokens=document["input_tokens"],
        output_tokens=document["output_tokens"],
        total_tokens=document["total_tokens"],
        input_token_details=realtime_types.RealtimeInputAudioTranscriptionUsageInputTokenDetails(
            text_tokens=document["text_tokens"], audio_tokens=document["audio_tokens"]
        ),
    )
    return cast(Json, dict(usage))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/realtime.py#L167-L187


def _duration_usage(document: UsageDocument) -> Json:
    duration = realtime_types.RealtimeInputAudioTranscriptionDurationUsage(
        type="duration", seconds=document["seconds"]
    )
    return cast(Json, dict(duration))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/realtime.py#L180-L183


def _records(input_value: Input) -> Json:
    records: list[Json] = []
    resolved: list[Json] = []
    for group in input_value["groups"]:
        kind = group["kind"]
        if kind == "usage":
            records.extend(
                _token_usage(item) if item["type"] == "tokens" else _duration_usage(item)
                for item in group["usage"]
            )
            continue
        for document in group["documents"]:
            if kind == "transcription_request":
                request = TranscriptionRequest.model_validate(document)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/realtime.py#L120-L142
                dumped = cast(Json, request.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/realtime.py#L60-L118
                resolved.append(request.resolved_model())
            else:
                dumped = cast(Json, _MODELS[kind].model_validate(document).model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/realtime.py#L60-L118
            records.append(dumped)
    return {"records": records, "resolved_models": resolved}


class ContractConfig(BaseRealtimeConfig):
    @override
    def validate_environment(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L43-L51
        self, headers: dict[str, Json], _model: str, _api_key: str | None = None
    ) -> dict[str, Json]:
        return dict(headers)

    @override
    def get_complete_url(self, api_base: str | None, model: str, api_key: str | None = None) -> str:
        return super().get_complete_url(api_base, model, api_key)

    @override
    def transform_realtime_request(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L71-L78
        self, _message: str, _model: str, _session_request: str | None = None
    ) -> list[str]:
        return []

    @override
    def transform_realtime_response(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L117-L127
        self,
        _message: str | bytes,
        _model: str,
        _logging_obj: LiteLLMLogging,
        _input: realtime_types.RealtimeResponseTransformInput,
    ) -> realtime_types.RealtimeResponseTypedDict:
        return cast(realtime_types.RealtimeResponseTypedDict, {})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L117-L127


class ContractHttpConfig(BaseRealtimeHTTPConfig):
    @override
    def get_api_base(self, api_base: str | None, **_kwargs: Json) -> str:
        return "" if api_base is None else api_base

    @override
    def get_api_key(self, api_key: str | None, **_kwargs: Json) -> str:
        return "" if api_key is None else api_key

    @override
    def get_complete_url(
        self, api_base: str | None, model: str, api_version: str | None = None
    ) -> str:
        return "" if api_base is None else api_base

    @override
    def validate_environment(
        self, headers: dict[str, Json], model: str, api_key: str | None = None
    ) -> dict[str, Json]:
        return dict(headers)


def _error_record(error: BaseLLMException, *, raised: bool) -> Json:
    headers = cast(dict[str, Json], error.headers)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L65
    return {"raised": raised, "status_code": error.status_code, "message": error.message,
            "headers": headers}


def _hooks(input_value: Input) -> Json:
    config = ContractConfig()
    error = input_value["error"]
    try:
        config.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L64-L69
            error["message"], error["status_code"], error["headers"]
        )
        raised: Json = None
    except BaseLLMException as exception:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L65
        raised = _error_record(exception, raised=True)
    returned: BaseLLMException = ContractHttpConfig().get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/http_transformation.py#L104-L117
        error["message"], error["status_code"], error["headers"]
    )
    created_event = cast(Json, config.transform_session_created_event("model", "session", None))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L103-L115
    return {
        "complete_urls": [
            config.get_complete_url(api_base, "model", None)
            for api_base in input_value["complete_urls"]
        ],
        "session_configuration_request": config.session_configuration_request("model"),
        "session_close_usage": cast(Json, config.unbilled_usage_on_session_close("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L97-L98
        "session_created_event": created_event,
        "is_setup_message": config.is_setup_message(_EMPTY),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L83-L91
        "is_content_message": config.is_content_message(_EMPTY),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/transformation.py#L83-L91
        "requires_session_configuration": config.requires_session_configuration(),
        "websocket_error": raised,
        "http_error": _error_record(returned, raised=False),
    }


def _url_record(config: ContractHttpConfig, document: UrlDocument) -> Json:
    header_map = config.get_realtime_calls_headers(document["ephemeral_key"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/http_transformation.py#L89-L98
    calls_headers = cast(dict[str, Json], header_map)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/realtime/http_transformation.py#L89-L98
    base, model, version = document["api_base"], document["model"], document["api_version"]
    record: dict[str, Json] = {"api_base": base, "model": model, "api_version": version}
    record["transcription"] = config.get_transcription_session_url(base, model, version)
    record["calls"] = config.get_realtime_calls_url(base, model, version)
    record["authorization"] = calls_headers["Authorization"]
    return record


def _backend_close(document: BackendError) -> Json:
    received, sent = (
        None if frame is None else Close(code=frame["code"], reason=frame["reason"])
        for frame in (document["received"], document["sent"])
    )
    error = ConnectionClosed(rcvd=received, sent=sent)
    close = backend_close_from(error)
    reason = None if received is None else close.reason
    is_error_text = close.reason == str(error)
    return {"code": close.code, "reason": reason, "reason_is_error_text": is_error_text}


def _close_helpers(input_value: Input) -> Json:
    event = realtime_errors.realtime_error_event(input_value["error"]["message"], "server_error")
    result: dict[str, Json] = {"error_event": cast(Json, json.loads(event))}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_errors.py#L26-L29
    result["close_reasons"] = [
        realtime_errors.websocket_close_reason(item["message"], item["fallback"])
        for item in input_value["reason_messages"]
    ]
    result["client_close_codes"] = [
        realtime_errors.client_close_code(code) for code in input_value["close_codes"]
    ]
    result["handshake_close_codes"] = [
        realtime_errors.upstream_handshake_close_code(status)
        for status in input_value["handshake_statuses"]
    ]
    result["backend_messages"] = [
        BackendClose(code=item["code"], reason=item["reason"]).message
        for item in input_value["backend_closes"]
    ]
    result["backend_closes"] = [_backend_close(item) for item in input_value["backend_errors"]]
    return result


def _cached_body_and_frames(input_value: Input) -> Json:
    bodies: list[Json] = [
        realtime_request_body(model).decode("utf-8") for model in input_value["models"]
    ]
    for index in range(1001):
        realtime_request_body(f"model-{index}")
    info = realtime_request_body.cache_info()
    frames = [cast(Json, decode_json(payload)) for payload in input_value["payloads"]]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/realtime_streaming.py#L106-L108
    return {
        "capacity": info.maxsize,
        "bodies": bodies,
        "size_after_distinct": info.currsize,
        "repeat_body": realtime_request_body("model-0").decode("utf-8"),
        "frames": frames,
    }


_HANDLERS: dict[str, Callable[[Input], Json]] = {
    "records": _records,
    "hooks": _hooks,
    "close_helpers": _close_helpers,
    "cached_body_and_frames": _cached_body_and_frames,
}


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "http_urls":
        config = ContractHttpConfig()
        return {"urls": [_url_record(config, document) for document in input_value["urls"]]}
    return _HANDLERS[function](input_value)
