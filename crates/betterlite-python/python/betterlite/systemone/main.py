from collections.abc import Mapping

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import CallValue, acall_native, call_native
from betterlite._native import NativeDecisionError, NativeDecisionSession
from betterlite.decisions.main import (
    JSON,
    QUESTIONS,
    SYSTEMONE,
    SYSTEMONE_RESPONSE,
    SystemOneQuestions,
    call_payload,
    decision_values,
    parse_response,
)
from betterlite.types.decisions import DecisionsJSON, DecisionsResponse


def systemone(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/systemone/main.py#L80
    model: str,
    state: DecisionsJSON | None = None,
    questions: SystemOneQuestions | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    extra_headers: Mapping[str, str] | None = None,
    safety_identifier: str | None = None,
    **kwargs: CallValue,
) -> DecisionsResponse:
    named: dict[str, CallValue] = {
        "model": model,
        "state": JSON.validate_python(state),
        "questions": JSON.validate_json(QUESTIONS.dump_json(questions)),
        "api_key": api_key,
        "api_base": api_base,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "safety_identifier": safety_identifier,
    }
    return parse_response(
        call_native(
            NativeDecisionSession("{}").call,
            call_payload(SYSTEMONE, decision_values(named, kwargs)),
            NativeDecisionError,
        ),
        SYSTEMONE_RESPONSE,
    )


async def asystemone(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/systemone/main.py#L52
    model: str,
    state: DecisionsJSON | None = None,
    questions: SystemOneQuestions | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    extra_headers: Mapping[str, str] | None = None,
    safety_identifier: str | None = None,
    **kwargs: CallValue,
) -> DecisionsResponse:
    named: dict[str, CallValue] = {
        "model": model,
        "state": JSON.validate_python(state),
        "questions": JSON.validate_json(QUESTIONS.dump_json(questions)),
        "api_key": api_key,
        "api_base": api_base,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
        "extra_headers": JSON.validate_python(extra_headers),
        "safety_identifier": safety_identifier,
    }
    payload = call_payload(SYSTEMONE, decision_values(named, kwargs))
    answer = await acall_native(NativeDecisionSession("{}").acall, payload, NativeDecisionError)
    return parse_response(answer, SYSTEMONE_RESPONSE)


__all__ = ["asystemone", "systemone"]
