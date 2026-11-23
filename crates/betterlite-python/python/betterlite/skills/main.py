from typing import Final, TypeAlias, TypeVar

from pydantic import BaseModel, JsonValue

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    to_call_payload,
)
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall
from betterlite._native import NativeSkillError, NativeSkillSession
from betterlite.types.llms.anthropic.anthropic_skills import (
    DeleteSkillResponse,
    ListSkillsResponse,
    Skill,
)

ENVIRONMENT: Final = "{}"
LOGGING_OBJECT: Final = "litellm_logging_obj"

Headers: TypeAlias = dict[str, str] | None
Query: TypeAlias = dict[str, JsonValue] | None
Model = TypeVar("Model", bound=BaseModel)


# LiteLLM's logging object belongs to its own runtime, so it never crosses into the payload
def _payload(**values: CallValue) -> CallPayload:
    named = {name: value for name, value in values.items() if name != LOGGING_OBJECT}
    return to_call_payload(named)


def _answer(model: type[Model], payload: CallPayload, method: NativeCall) -> Model:
    answer = call_native(method, payload, NativeSkillError)
    return model.model_validate(answer)


async def _aanswer(model: type[Model], payload: CallPayload, method: AsyncNativeCall) -> Model:
    answer = await acall_native(method, payload, NativeSkillError)
    return model.model_validate(answer)


def create_skill(files: list[CallValue] | None = None, display_title: str | None = None,
                 extra_headers: Headers = None, extra_query: Query = None,
                 extra_body: Query = None, **kwargs: CallValue) -> Skill:
    payload = _payload(files=files, display_title=display_title, extra_headers=extra_headers,
                       extra_query=extra_query, extra_body=extra_body, **kwargs)
    return _answer(Skill, payload, NativeSkillSession(ENVIRONMENT).create_skill)


async def acreate_skill(files: list[CallValue] | None = None, display_title: str | None = None,
                        extra_headers: Headers = None, extra_query: Query = None,
                        extra_body: Query = None, **kwargs: CallValue) -> Skill:
    payload = _payload(files=files, display_title=display_title, extra_headers=extra_headers,
                       extra_query=extra_query, extra_body=extra_body, **kwargs)
    return await _aanswer(Skill, payload, NativeSkillSession(ENVIRONMENT).acreate_skill)


def list_skills(limit: int | None = None, page: str | None = None, source: str | None = None,
                extra_headers: Headers = None, extra_query: Query = None,
                **kwargs: CallValue) -> ListSkillsResponse:
    payload = _payload(limit=limit, page=page, source=source, extra_headers=extra_headers,
                       extra_query=extra_query, **kwargs)
    return _answer(ListSkillsResponse, payload, NativeSkillSession(ENVIRONMENT).list_skills)


async def alist_skills(limit: int | None = None, page: str | None = None,
                       source: str | None = None, extra_headers: Headers = None,
                       extra_query: Query = None, **kwargs: CallValue) -> ListSkillsResponse:
    payload = _payload(limit=limit, page=page, source=source, extra_headers=extra_headers,
                       extra_query=extra_query, **kwargs)
    return await _aanswer(ListSkillsResponse, payload,
                          NativeSkillSession(ENVIRONMENT).alist_skills)


def get_skill(skill_id: str, extra_headers: Headers = None, extra_query: Query = None,
              timeout: float | TimeoutValues | None = None,
              custom_llm_provider: str | None = None, **kwargs: CallValue) -> Skill:
    payload = _payload(skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query,
                       timeout=timeout, custom_llm_provider=custom_llm_provider, **kwargs)
    return _answer(Skill, payload, NativeSkillSession(ENVIRONMENT).get_skill)


async def aget_skill(skill_id: str, extra_headers: Headers = None, extra_query: Query = None,
                     **kwargs: CallValue) -> Skill:
    payload = _payload(skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query,
                       **kwargs)
    return await _aanswer(Skill, payload, NativeSkillSession(ENVIRONMENT).aget_skill)


def delete_skill(skill_id: str, extra_headers: Headers = None,
                 extra_query: Query = None, timeout: float | TimeoutValues | None = None,
                 custom_llm_provider: str | None = None,
                 **kwargs: CallValue) -> DeleteSkillResponse:
    payload = _payload(skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query,
                       timeout=timeout, custom_llm_provider=custom_llm_provider, **kwargs)
    return _answer(DeleteSkillResponse, payload, NativeSkillSession(ENVIRONMENT).delete_skill)


async def adelete_skill(skill_id: str, extra_headers: Headers = None,
                        extra_query: Query = None, **kwargs: CallValue) -> DeleteSkillResponse:
    payload = _payload(skill_id=skill_id, extra_headers=extra_headers, extra_query=extra_query,
                       **kwargs)
    return await _aanswer(DeleteSkillResponse, payload,
                          NativeSkillSession(ENVIRONMENT).adelete_skill)
