from collections.abc import Awaitable, Mapping
from typing import Final, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite import exceptions
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._cost_extra import rerank_cost
from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    to_call_payload,
)
from betterlite._native import NativeRerankError, NativeRerankSession
from betterlite._rerank.limits import RequestBudget, check_request_size, rerank_settings
from betterlite.types.rerank import RerankResponse, RerankResponseMeta, RerankResponseResult

MISSING_MODEL: Final = (
    "model parameter is required but was None. Please provide a valid model name."
)
MAX_DOCUMENTS: Final = 100_000
# every argument rerank names itself: these never travel as an unknown provider parameter
NAMED_ARGUMENTS: Final[tuple[str, ...]] = (
    "custom_llm_provider", "documents", "max_chunks_per_doc", "max_tokens_per_doc", "model",
    "query", "rank_fields", "return_documents", "top_n",
)
# arerank declares every name but max_tokens_per_doc: it stays a kwarg and binds rerank's parameter
ASYNC_NAMED_ARGUMENTS: Final[tuple[str, ...]] = (
    "custom_llm_provider", "documents", "max_chunks_per_doc", "model", "query", "rank_fields",
    "return_documents", "top_n",
)
# the named arguments Rust reads; a None one stays out so its default applies there
OPTIONAL_ARGUMENTS: Final[tuple[str, ...]] = (
    "custom_llm_provider", "max_chunks_per_doc", "max_tokens_per_doc", "rank_fields",
    "return_documents", "top_n",
)
# LiteLLM's own bookkeeping and the clients Rust cannot use; the payload drops or refuses them
SKIPPED_KWARGS: Final[tuple[str, ...]] = (
    "_is_async", "arerank", "azure_ad_token_provider", "client", "litellm_logging_obj",
    "logger_fn", "shared_session",
)

TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
HEADERS: Final[TypeAdapter[dict[str, JsonValue] | None]] = TypeAdapter(dict[str, JsonValue] | None)


class RerankAnswer(TypedDict):
    id: str | None
    results: list[RerankResponseResult] | None
    meta: RerankResponseMeta | None
    provider: str
    hidden_params: dict[str, JsonValue]
    response_ms: float


ANSWER: Final[TypeAdapter[RerankAnswer]] = TypeAdapter(RerankAnswer)


def rerank(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/main.py#L93
    model: str | None,
    query: str,
    documents: list[str | dict[str, JsonValue]],
    custom_llm_provider: str | None = None,
    top_n: int | None = None,
    rank_fields: list[str] | None = None,
    return_documents: bool | None = True,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/main.py#L114
    max_chunks_per_doc: int | None = None,
    max_tokens_per_doc: int | None = None,
    **kwargs: CallValue,
) -> RerankResponse | Awaitable[RerankResponse]:
    check_request_size((
        model, query, documents, custom_llm_provider, top_n, rank_fields,
        return_documents, max_chunks_per_doc, max_tokens_per_doc,
    ), kwargs)
    values: dict[str, CallValue] = dict(kwargs)
    values |= request_arguments(
        model, query, documents, custom_llm_provider, top_n, rank_fields,
        max_chunks_per_doc, return_documents=return_documents,
    )
    values["max_tokens_per_doc"] = max_tokens_per_doc
    check_model(values)
    if values.get("arerank") is True or values.get("_is_async") is True:
        return rerank_async(values, ASYNC_NAMED_ARGUMENTS)
    return rerank_sync(values, NAMED_ARGUMENTS)


async def arerank(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/main.py#L31
    model: str | None,
    query: str,
    documents: list[str | dict[str, JsonValue]],
    custom_llm_provider: str | None = None,
    top_n: int | None = None,
    rank_fields: list[str] | None = None,
    return_documents: bool | None = None,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/main.py#L40
    max_chunks_per_doc: int | None = None,
    **kwargs: CallValue,
) -> RerankResponse:
    check_request_size((
        model, query, documents, custom_llm_provider, top_n, rank_fields,
        return_documents, max_chunks_per_doc,
    ), kwargs)
    values: dict[str, CallValue] = kwargs | request_arguments(
        model, query, documents, custom_llm_provider, top_n, rank_fields,
        max_chunks_per_doc, return_documents=return_documents,
    )
    check_model(values)
    return await rerank_async(values, ASYNC_NAMED_ARGUMENTS)


def check_model(values: Mapping[str, CallValue]) -> None:
    if TEXT.validate_python(values.get("model")) is None:
        raise exceptions.BadRequestError(
            f"GetLLMProvider Exception - {MISSING_MODEL}\n\noriginal model: None", None, ""
        )


def request_arguments(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/main.py#L164-L178
    model: str | None,
    query: str,
    documents: list[str | dict[str, JsonValue]],
    custom_llm_provider: str | None,
    top_n: int | None,
    rank_fields: list[str] | None,
    max_chunks_per_doc: int | None,
    *,
    return_documents: bool | None,
) -> dict[str, CallValue]:
    if len(documents) > MAX_DOCUMENTS:
        raise ValueError(f"too many rerank documents: {len(documents)} > {MAX_DOCUMENTS}")
    return {
        "model": model,
        "query": query,
        "documents": JSON.validate_python(documents),
        "custom_llm_provider": custom_llm_provider,
        "top_n": top_n,
        "rank_fields": JSON.validate_python(rank_fields),
        "return_documents": return_documents,
        "max_chunks_per_doc": max_chunks_per_doc,
    }


def rerank_sync(values: Mapping[str, CallValue], named: tuple[str, ...]) -> RerankResponse:
    answer = call_native(
        NativeRerankSession("{}").rerank, call_payload(values, named), NativeRerankError
    )
    return response_from_answer(ANSWER.validate_python(answer))


async def rerank_async(
    values: Mapping[str, CallValue], named: tuple[str, ...]
) -> RerankResponse:
    answer = await acall_native(
        NativeRerankSession("{}").arerank, call_payload(values, named), NativeRerankError
    )
    return response_from_answer(ANSWER.validate_python(answer))


def response_from_answer(answer: RerankAnswer) -> RerankResponse:
    response = RerankResponse.model_validate(answer)
    hidden = dict(answer["hidden_params"])
    try:
        model = TEXT.validate_python(hidden.get("litellm_model_name"))
        meta = response.meta
        billed = None if meta is None else meta.get("billed_units")
        if model is None:
            hidden["response_cost"] = None
        else:
            prompt_cost, completion_cost = rerank_cost(model, answer["provider"], billed)
            hidden["response_cost"] = prompt_cost + completion_cost
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L1940
        hidden["response_cost"] = None
    response["_hidden_params"] = hidden
    return response


def call_payload(values: Mapping[str, CallValue], named: tuple[str, ...]) -> CallPayload:
    # the top-level names are the payload's own: it drops the logger, refuses a client and warns
    settings = rerank_settings()
    check_request_size((), values)
    _ = HEADERS.validate_python(values.get("headers"))
    _ = HEADERS.validate_python(settings.get("headers"))
    payload: dict[str, CallValue] = {
        "arguments": forwarded_arguments(values, named),
        "kwargs": extra_arguments(values, named),
        "settings": settings,
        "client": values.get("client"),
        "logger_fn": values.get("logger_fn"),
        "shared_session": values.get("shared_session"),
    }
    budget = RequestBudget()
    budget.consume_fields(payload)
    return to_call_payload(payload)


# the route's own declared names decide which optional arguments bind here; the rest stay in kwargs
def forwarded_arguments(
    values: Mapping[str, CallValue], named: tuple[str, ...]
) -> dict[str, CallValue]:
    if values.get("azure_ad_token_provider") is not None:
        raise UnsupportedCallArgument("azure_ad_token_provider")
    documents = JSON.validate_python(values.get("documents"))
    model = TEXT.validate_python(values.get("model"))
    query = TEXT.validate_python(values.get("query"))
    forwarded: dict[str, CallValue] = {
        "model": "" if model is None else model,
        "query": "" if query is None else query,
        "documents": documents,
    }
    for name in OPTIONAL_ARGUMENTS:
        value = values.get(name)
        if value is not None and name in named:
            forwarded[name] = value
    return forwarded


def extra_arguments(
    values: Mapping[str, CallValue], named: tuple[str, ...]
) -> dict[str, CallValue]:
    return {
        name: value
        for name, value in values.items()
        if value is not None and name not in named and name not in SKIPPED_KWARGS
    }
