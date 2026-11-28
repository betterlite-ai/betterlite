import json
from collections.abc import Awaitable
from typing import Final, TypeAlias, TypeVar

from pydantic import BaseModel, JsonValue, TypeAdapter

from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall, acall_native, call_native
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeEvalError, NativeEvalSession
from betterlite.types.llms.openai_evals import (
    CancelEvalResponse,
    DataSourceConfig,
    DeleteEvalResponse,
    Eval,
    GraderConfig,
    ListEvalsResponse,
)

SETTINGS: Final[tuple[str, ...]] = ("api_key", "openai_key")
LOGGING_OBJECT: Final = "litellm_logging_obj"

Headers: TypeAlias = dict[str, str] | None
Query: TypeAlias = dict[str, JsonValue] | None
Body: TypeAlias = dict[str, JsonValue] | None
Model = TypeVar("Model", bound=BaseModel)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def _session() -> NativeEvalSession:
    settings: dict[str, JsonValue] = {"settings": read_settings(SETTINGS)}
    return NativeEvalSession(json.dumps(settings))


# LiteLLM's logging object belongs to its own runtime, so it never crosses into the payload
def _payload(
    extra_headers: Headers, extra_query: Query, timeout: float | TimeoutValues | None,
    custom_llm_provider: str | None, **values: CallValue,
) -> CallPayload:
    named = {name: value for name, value in values.items() if name != LOGGING_OBJECT}
    shared: dict[str, CallValue] = {
        "extra_headers": extra_headers, "extra_query": extra_query,
        "timeout": timeout, "custom_llm_provider": custom_llm_provider,
    }
    named.update(shared)
    return to_call_payload(named)


def _answer(model: type[Model], payload: CallPayload, method: NativeCall) -> Model:
    answer = call_native(method, payload, NativeEvalError)
    return model.model_validate(answer)


async def _aanswer(model: type[Model], payload: CallPayload, method: AsyncNativeCall) -> Model:
    answer = await acall_native(method, payload, NativeEvalError)
    return model.model_validate(answer)


# the sync twin answers a coroutine when its async keyword is set, and its result itself otherwise
async def _awaited(pending: Model | Awaitable[Model]) -> Model:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


def create_eval(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L114
    data_source_config: DataSourceConfig,
    testing_criteria: list[GraderConfig],
    name: str | None = None, metadata: dict[str, JsonValue] | None = None,
    extra_headers: Headers = None, extra_query: Query = None, extra_body: Body = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Eval | Awaitable[Eval]:
    asynchronous = kwargs.pop("acreate_eval", None) is True
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        data_source_config=JSON.validate_python(data_source_config),
        testing_criteria=JSON.validate_python(testing_criteria),
        name=name, metadata=metadata, extra_body=extra_body, **kwargs,
    )
    match asynchronous:
        case True:
            return _aanswer(Eval, payload, _session().acreate_eval)
        case False:
            return _answer(Eval, payload, _session().create_eval)


async def acreate_eval(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L45
    data_source_config: DataSourceConfig,
    testing_criteria: list[GraderConfig],
    name: str | None = None, metadata: dict[str, JsonValue] | None = None,
    extra_headers: Headers = None, extra_query: Query = None, extra_body: Body = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L53
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Eval:
    kwargs["acreate_eval"] = True
    pending = create_eval(
        data_source_config, testing_criteria, name, metadata, extra_headers, extra_query,
        extra_body, timeout, custom_llm_provider, **kwargs,
    )
    return await _awaited(pending)


def list_evals(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L299
    limit: int | None = None, after: str | None = None, before: str | None = None,
    order: str | None = None, order_by: str | None = None,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> ListEvalsResponse | Awaitable[ListEvalsResponse]:
    asynchronous = kwargs.pop("alist_evals", None) is True
    pagination: dict[str, CallValue] = {
        "limit": limit, "after": after, "before": before,
        "order": order, "order_by": order_by,
    }
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider, **pagination, **kwargs
    )
    session = _session()
    return (
        _aanswer(ListEvalsResponse, payload, session.alist_evals)
        if asynchronous
        else _answer(ListEvalsResponse, payload, session.list_evals)
    )


async def alist_evals(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L230
    limit: int | None = None, after: str | None = None, before: str | None = None,
    order: str | None = None, order_by: str | None = None,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L238
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> ListEvalsResponse:
    pagination: dict[str, CallValue] = {
        "limit": limit, "after": after, "before": before,
        "order": order, "order_by": order_by,
    }
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider, **pagination, **kwargs
    )
    answer = await acall_native(_session().alist_evals, payload, NativeEvalError)
    return ListEvalsResponse.model_validate(answer)


def get_eval(
    eval_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Eval | Awaitable[Eval]:
    if kwargs.pop("aget_eval", False) is True:
        return aget_eval(
            eval_id, extra_headers, extra_query, timeout, custom_llm_provider, **kwargs
        )
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, **kwargs,
    )
    return _answer(Eval, payload, _session().get_eval)


async def aget_eval(
    eval_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L420
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Eval:
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, **kwargs,
    )
    return await _aanswer(Eval, payload, _session().aget_eval)


def update_eval(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L632
    eval_id: str,
    name: str | None = None, metadata: dict[str, JsonValue] | None = None,
    extra_headers: Headers = None, extra_query: Query = None, extra_body: Body = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Eval | Awaitable[Eval]:
    asynchronous = kwargs.pop("aupdate_eval", None) is True
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, name=name, metadata=metadata, extra_body=extra_body, **kwargs,
    )
    if asynchronous:
        return _aanswer(Eval, payload, _session().aupdate_eval)
    return _answer(Eval, payload, _session().update_eval)


async def aupdate_eval(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L566
    eval_id: str,
    name: str | None = None, metadata: dict[str, JsonValue] | None = None,
    extra_headers: Headers = None, extra_query: Query = None, extra_body: Body = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L573
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Eval:
    kwargs.update(aupdate_eval=True)
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, name=name, metadata=metadata, extra_body=extra_body, **kwargs,
    )
    return await _aanswer(Eval, payload, _session().aupdate_eval)


def delete_eval(
    eval_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> DeleteEvalResponse | Awaitable[DeleteEvalResponse]:
    if kwargs.pop("adelete_eval", False) is True:
        return adelete_eval(
            eval_id, extra_headers, extra_query, timeout, custom_llm_provider, **kwargs
        )
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, **kwargs,
    )
    answer = call_native(_session().delete_eval, payload, NativeEvalError)
    return DeleteEvalResponse.model_validate(answer)


async def adelete_eval(
    eval_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L790
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> DeleteEvalResponse:
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, **kwargs,
    )
    answer = await acall_native(_session().adelete_eval, payload, NativeEvalError)
    return DeleteEvalResponse.model_validate(answer)


def cancel_eval(
    eval_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> CancelEvalResponse | Awaitable[CancelEvalResponse]:
    asynchronous = kwargs.pop("acancel_eval", None) is True
    payload = _payload(
        extra_headers, extra_query, timeout, custom_llm_provider,
        eval_id=eval_id, **kwargs,
    )
    session = _session()
    match asynchronous:
        case True:
            return _aanswer(CancelEvalResponse, payload, session.acancel_eval)
        case False:
            return _answer(CancelEvalResponse, payload, session.cancel_eval)


async def acancel_eval(
    eval_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L940
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> CancelEvalResponse:
    kwargs["acancel_eval"] = True
    pending = cancel_eval(
        eval_id, extra_headers, extra_query, timeout, custom_llm_provider, **kwargs
    )
    return await _awaited(pending)
