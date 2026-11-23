from typing import Final

from pydantic import JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    to_call_payload,
)
from betterlite._native import NativeSkillError, NativeSkillSession
from betterlite.types.llms.anthropic.anthropic_skills import (
    DeleteSkillResponse,
    ListSkillsResponse,
    Skill,
)

ENVIRONMENT: Final = "{}"
LOGGING_OBJECT: Final = "litellm_logging_obj"


# LiteLLM's logging object belongs to its own runtime, so it never crosses into the payload
def _payload(**values: CallValue) -> CallPayload:
    named = {name: value for name, value in values.items() if name != LOGGING_OBJECT}
    return to_call_payload(named)


def create_skill(
    files: list[CallValue] | None = None,
    display_title: str | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> Skill:
    payload = _payload(
        files=files, display_title=display_title, extra_headers=extra_headers,
        extra_query=extra_query, extra_body=extra_body, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = call_native(session.create_skill, payload, NativeSkillError)
    return Skill.model_validate(answer)


async def acreate_skill(
    files: list[CallValue] | None = None,
    display_title: str | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    extra_body: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> Skill:
    payload = _payload(
        files=files, display_title=display_title, extra_headers=extra_headers,
        extra_query=extra_query, extra_body=extra_body, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = await acall_native(session.acreate_skill, payload, NativeSkillError)
    return Skill.model_validate(answer)


def list_skills(
    limit: int | None = None,
    page: str | None = None,
    source: str | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ListSkillsResponse:
    payload = _payload(
        limit=limit, page=page, source=source, extra_headers=extra_headers,
        extra_query=extra_query, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = call_native(session.list_skills, payload, NativeSkillError)
    return ListSkillsResponse.model_validate(answer)


async def alist_skills(
    limit: int | None = None,
    page: str | None = None,
    source: str | None = None,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> ListSkillsResponse:
    payload = _payload(
        limit=limit, page=page, source=source, extra_headers=extra_headers,
        extra_query=extra_query, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = await acall_native(session.alist_skills, payload, NativeSkillError)
    return ListSkillsResponse.model_validate(answer)


def get_skill(
    skill_id: str,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Skill:
    payload = _payload(
        skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query,
        timeout=timeout, custom_llm_provider=custom_llm_provider, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = call_native(session.get_skill, payload, NativeSkillError)
    return Skill.model_validate(answer)


async def aget_skill(
    skill_id: str,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> Skill:
    payload = _payload(
        skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = await acall_native(session.aget_skill, payload, NativeSkillError)
    return Skill.model_validate(answer)


def delete_skill(
    skill_id: str,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> DeleteSkillResponse:
    payload = _payload(
        skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query,
        timeout=timeout, custom_llm_provider=custom_llm_provider, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = call_native(session.delete_skill, payload, NativeSkillError)
    return DeleteSkillResponse.model_validate(answer)


async def adelete_skill(
    skill_id: str,
    extra_headers: dict[str, str] | None = None,
    extra_query: dict[str, JsonValue] | None = None,
    **kwargs: CallValue,
) -> DeleteSkillResponse:
    payload = _payload(
        skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query, **kwargs,
    )
    session = NativeSkillSession(ENVIRONMENT)
    answer = await acall_native(session.adelete_skill, payload, NativeSkillError)
    return DeleteSkillResponse.model_validate(answer)
