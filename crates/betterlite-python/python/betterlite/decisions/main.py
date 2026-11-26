import warnings
from collections.abc import Mapping, Sequence
from typing import Final, NoReturn, Protocol, TypeAlias, TypeVar

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import TimeoutValues
from betterlite._completion.errors import (
    ERROR_PAYLOAD,
    ErrorPayload,
    raise_native_error,
)
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._native import (
    NativeDecisionError,
    NativeDecisionSession,
)
from betterlite._native import (
    supported_decisions_providers as provider_names,
)
from betterlite.types.decisions import (
    DecisionQuestion,
    DecisionsResponse,
    OpenAIDecisionInput,
    OpenAIDecisionQuestion,
    OpenAIDecisionResponse,
)

SETTINGS: Final[tuple[str, ...]] = (
    "api_base",
    "api_key",
    "drop_params",
    "openai_key",
    "organization",
)
BOOKKEEPING: Final[tuple[str, ...]] = (
    "client",
    "litellm_logging_obj",
    "logger_fn",
    "shared_session",
)
DECISIONS: Final = "decisions"
SYSTEMONE: Final = "systemone"
STATE: Final = "state"
DEPRECATION: Final = (
    "litellm.decisions(state=...) is deprecated, call litellm.systemone for the "
    "System One request format"
)

OpenAIDecisionQuestions: TypeAlias = Sequence[
    OpenAIDecisionQuestion | Mapping[str, JsonValue]
]
SystemOneQuestions: TypeAlias = Mapping[str, DecisionQuestion | Mapping[str, JsonValue]]

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
OPENAI_RESPONSE: Final[TypeAdapter[OpenAIDecisionResponse]] = TypeAdapter(OpenAIDecisionResponse)
SYSTEMONE_RESPONSE: Final[TypeAdapter[DecisionsResponse]] = TypeAdapter(DecisionsResponse)


class HiddenResponse(Protocol):
    def set_hidden_params(self, params: Mapping[str, JsonValue]) -> None: ...


ResponseT = TypeVar("ResponseT", bound=HiddenResponse)


# LiteLLM's sandbox providers raise builtins the exception table has no class for
def raise_decision_error(error: NativeDecisionError) -> NoReturn:
    payload: ErrorPayload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] == "ValueError":
        raise ValueError(payload.get("message")) from error
    if payload["type"] == "TimeoutError":
        raise TimeoutError(payload.get("message")) from error
    raise_native_error(error)


def parse_response(answer: JsonValue, adapter: TypeAdapter[ResponseT]) -> ResponseT:
    fields = OBJECT.validate_python(answer)
    hidden = OBJECT.validate_python(fields.pop("_hidden_params", {}))
    response = adapter.validate_python(fields)
    response.set_hidden_params(hidden)
    return response


# a call that carried the deprecated state answers in the System One format
def decision_response(
    answer: JsonValue, kwargs: Mapping[str, CallValue]
) -> OpenAIDecisionResponse | DecisionsResponse:
    if STATE in kwargs:
        return parse_response(answer, SYSTEMONE_RESPONSE)
    return parse_response(answer, OPENAI_RESPONSE)


def call_payload(operation: str, values: Mapping[str, CallValue]) -> CallPayload:
    arguments = {name: value for name, value in values.items() if name not in BOOKKEEPING}
    payload: dict[str, CallValue] = {
        "arguments": arguments,
        "settings": read_settings(SETTINGS),
        "operation": operation,
        "client": values.get("client"),
        "logger_fn": values.get("logger_fn"),
        "shared_session": values.get("shared_session"),
    }
    return to_call_payload(payload)


# the state keyword is LiteLLM's deprecated System One request format, passed on as a param
def decision_values(
    named: Mapping[str, CallValue], kwargs: Mapping[str, CallValue]
) -> dict[str, CallValue]:
    params = {name: value for name, value in kwargs.items() if name not in BOOKKEEPING}
    values: dict[str, CallValue] = {**named, "params": params}
    values |= {name: kwargs[name] for name in BOOKKEEPING if name in kwargs}
    return values


# ast-grep-ignore: no-one-line-functions - call.py's entrypoint re-exports the native provider list
def supported_decisions_providers() -> tuple[str, ...]:
    return tuple(provider_names())


def decisions(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L104
    model: str,
    input: OpenAIDecisionInput | None = None,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L106
    questions: OpenAIDecisionQuestions | SystemOneQuestions | None = None,
    safety_identifier: str | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    extra_headers: Mapping[str, str] | None = None,
    **kwargs: CallValue,
) -> OpenAIDecisionResponse | DecisionsResponse:
    if STATE in kwargs:
        warnings.warn(DEPRECATION, UserWarning, stacklevel=2)
    named: dict[str, CallValue] = {
        "model": model,
        "input": JSON.validate_python(input),
        "questions": JSON.validate_python(questions),
        "safety_identifier": safety_identifier,
        "api_key": api_key,
        "api_base": api_base,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
    }
    return decision_response(
        call_native(
            NativeDecisionSession("{}").call,
            call_payload(DECISIONS, decision_values(named, kwargs)),
            NativeDecisionError,
        ),
        kwargs,
    )


async def adecisions(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L76
    model: str,
    input: OpenAIDecisionInput | None = None,  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L78
    questions: OpenAIDecisionQuestions | SystemOneQuestions | None = None,
    safety_identifier: str | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/decisions/main.py#L83
    custom_llm_provider: str | None = None,
    extra_headers: Mapping[str, str] | None = None,
    **kwargs: CallValue,
) -> OpenAIDecisionResponse | DecisionsResponse:
    deprecated = STATE in kwargs
    if deprecated:
        warnings.warn(DEPRECATION, UserWarning, stacklevel=2)
    named: dict[str, CallValue] = {
        "model": model,
        "input": JSON.validate_python(input),
        "questions": JSON.validate_python(questions),
        "safety_identifier": safety_identifier,
        "api_key": api_key,
        "api_base": api_base,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
    }
    payload = call_payload(DECISIONS, decision_values(named, kwargs))
    answer = await acall_native(NativeDecisionSession("{}").acall, payload, NativeDecisionError)
    return decision_response(answer, kwargs)


__all__ = ["adecisions", "decisions", "supported_decisions_providers"]
