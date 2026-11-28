import json
from collections.abc import Awaitable
from typing import Final, TypeAlias, TypeVar

from pydantic import BaseModel, JsonValue, TypeAdapter
from typing_extensions import TypedDict

import betterlite
from betterlite._completion.arguments import TimeoutValues
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import AsyncNativeCall, NativeCall, acall_native, call_native
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeEvalRunError, NativeEvalRunSession
from betterlite.types.llms.openai_evals import (
    CancelRunResponse,
    ListRunsResponse,
    Run,
    RunDeleteResponse,
)

SETTINGS: Final[tuple[str, ...]] = ("api_key", "openai_key")
LOGGING_OBJECT: Final = "litellm_logging_obj"
CREDENTIAL_NAME: Final = "litellm_credential_name"

Headers: TypeAlias = dict[str, str] | None
Query: TypeAlias = dict[str, JsonValue] | None
Body: TypeAlias = dict[str, JsonValue] | None
Model = TypeVar("Model", bound=BaseModel)


class CredentialEntry(TypedDict):
    credential_name: str
    credential_values: dict[str, JsonValue]


CREDENTIALS: Final[TypeAdapter[list[CredentialEntry]]] = TypeAdapter(list[CredentialEntry])


def _session() -> NativeEvalRunSession:
    settings: dict[str, JsonValue] = {"settings": read_settings(SETTINGS)}
    return NativeEvalRunSession(json.dumps(settings))


# the four parameters every run call shares
def _shared(
    extra_headers: Headers, extra_query: Query, timeout: float | TimeoutValues | None,
    custom_llm_provider: str | None,
) -> dict[str, CallValue]:
    return {
        "extra_headers": extra_headers, "extra_query": extra_query,
        "timeout": timeout, "custom_llm_provider": custom_llm_provider,
    }


# LiteLLM's wrapper fills kwargs from a matching litellm_credential_name before the body runs
def _with_credentials(values: dict[str, CallValue]) -> None:
    name = values.get(CREDENTIAL_NAME)
    match name:
        case str() if not name:
            return
        case str():
            for entry in CREDENTIALS.validate_python(betterlite.credential_list):
                if entry["credential_name"] == name:
                    for key, value in entry["credential_values"].items():
                        values.setdefault(key, value)
                    return
        case object():
            return


# LiteLLM's logging object belongs to its own runtime, so it never crosses into the payload
def _payload(named: dict[str, CallValue], shared: dict[str, CallValue]) -> CallPayload:
    values = {name: value for name, value in named.items() if name != LOGGING_OBJECT}
    values.update({name: value for name, value in shared.items() if value is not None})
    _with_credentials(values)
    return to_call_payload(values)


def _answer(model: type[Model], payload: CallPayload, method: NativeCall) -> Model:
    answer = call_native(method, payload, NativeEvalRunError)
    return model.model_validate(answer)


async def _aanswer(model: type[Model], payload: CallPayload, method: AsyncNativeCall) -> Model:
    answer = await acall_native(method, payload, NativeEvalRunError)
    return model.model_validate(answer)


# the sync twin answers a coroutine when its async keyword is set, and its result itself otherwise
async def _awaited(pending: Model | Awaitable[Model]) -> Model:
    match pending:
        case Awaitable():
            return await pending
        case object():
            return pending


def create_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1163
    eval_id: str,
    data_source: dict[str, JsonValue],
    name: str | None = None, metadata: dict[str, JsonValue] | None = None,
    extra_headers: Headers = None, extra_query: Query = None, extra_body: Body = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Run | Awaitable[Run]:
    asynchronous = kwargs.pop("acreate_run", None) is True
    payload = _payload(
        {"eval_id": eval_id, "data_source": data_source, "name": name,
         "metadata": metadata, "extra_body": extra_body, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    session = _session()
    match asynchronous:
        case True:
            return _aanswer(Run, payload, session.acreate_run)
        case False:
            return _answer(Run, payload, session.create_run)


async def acreate_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1095
    eval_id: str,
    data_source: dict[str, JsonValue],
    name: str | None = None, metadata: dict[str, JsonValue] | None = None,
    extra_headers: Headers = None, extra_query: Query = None, extra_body: Body = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1103
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Run:
    kwargs["acreate_run"] = True
    pending = create_run(
        eval_id, data_source, name, metadata, extra_headers, extra_query,
        extra_body, timeout, custom_llm_provider, **kwargs,
    )
    return await _awaited(pending)


def list_runs(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1347
    eval_id: str,
    limit: int | None = None, after: str | None = None, before: str | None = None,
    order: str | None = None,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> ListRunsResponse | Awaitable[ListRunsResponse]:
    asynchronous = kwargs.pop("alist_runs", None) is True
    pagination: dict[str, CallValue] = {
        "eval_id": eval_id, "limit": limit, "after": after, "before": before,
        "order": order,
    }
    payload = _payload(
        {**pagination, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    session = _session()
    if asynchronous:
        return _aanswer(ListRunsResponse, payload, session.alist_runs)
    return _answer(ListRunsResponse, payload, session.list_runs)


async def alist_runs(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1278
    eval_id: str,
    limit: int | None = None, after: str | None = None, before: str | None = None,
    order: str | None = None,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1287
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> ListRunsResponse:
    pagination: dict[str, CallValue] = {
        "eval_id": eval_id, "limit": limit, "after": after, "before": before,
        "order": order,
    }
    payload = _payload(
        {**pagination, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    return await _aanswer(ListRunsResponse, payload, _session().alist_runs)


def get_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1523
    eval_id: str,
    run_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Run | Awaitable[Run]:
    if kwargs.pop("aget_run", False) is True:
        return aget_run(
            eval_id, run_id, extra_headers, extra_query, timeout, custom_llm_provider, **kwargs
        )
    payload = _payload(
        {"eval_id": eval_id, "run_id": run_id, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    return _answer(Run, payload, _session().get_run)


async def aget_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1463
    eval_id: str,
    run_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1469
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> Run:
    payload = _payload(
        {"eval_id": eval_id, "run_id": run_id, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    return await _aanswer(Run, payload, _session().aget_run)


def cancel_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1679
    eval_id: str,
    run_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> CancelRunResponse | Awaitable[CancelRunResponse]:
    asynchronous = kwargs.pop("acancel_run", None) is True
    payload = _payload(
        {"eval_id": eval_id, "run_id": run_id, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    if asynchronous:
        return _aanswer(CancelRunResponse, payload, _session().acancel_run)
    return _answer(CancelRunResponse, payload, _session().cancel_run)


async def acancel_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1619
    eval_id: str,
    run_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1625
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> CancelRunResponse:
    kwargs["acancel_run"] = True
    pending: CancelRunResponse | Awaitable[CancelRunResponse] = cancel_run(
        eval_id, run_id, extra_headers, extra_query, timeout, custom_llm_provider, **kwargs
    )
    return await _awaited(pending)


def delete_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1844
    eval_id: str,
    run_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> RunDeleteResponse | Awaitable[RunDeleteResponse]:
    asynchronous = kwargs.pop("adelete_run", False) is True
    if asynchronous:
        return adelete_run(
            eval_id, run_id, extra_headers, extra_query, timeout, custom_llm_provider, **kwargs
        )
    payload = _payload(
        {"eval_id": eval_id, "run_id": run_id, **kwargs},
        _shared(extra_headers, extra_query, timeout, custom_llm_provider),
    )
    return _answer(RunDeleteResponse, payload, _session().delete_run)


async def adelete_run(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1784
    eval_id: str,
    run_id: str,
    extra_headers: Headers = None, extra_query: Query = None,
    timeout: float | TimeoutValues | None = None,  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/evals/main.py#L1790
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> RunDeleteResponse:
    shared = _shared(extra_headers, extra_query, timeout, custom_llm_provider)
    payload = _payload({"eval_id": eval_id, "run_id": run_id, **kwargs}, shared)
    return await _aanswer(RunDeleteResponse, payload, _session().adelete_run)
