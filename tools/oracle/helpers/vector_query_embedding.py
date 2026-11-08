import asyncio
import json
from collections.abc import Callable, Mapping, Sequence
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.vector_store.transformation import (
    BaseDirectVectorStoreConfig,
    BaseQueryEmbeddingVectorStoreConfig,
    LiteLLMVectorStoreEmbeddingExecutor,
    RouterVectorStoreEmbeddingExecutor,
)
from litellm.types.vector_stores import VectorStoreSearchOptionalRequestParams
from pydantic import ValidationError

if TYPE_CHECKING:
    import httpx
    from litellm.router import Router

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class Recording:
    def __init__(self) -> None:
        self.model_call_details: dict[str, object] = {}


def recording() -> Logging:
    return cast("Logging", Recording())


class Fixture(BaseQueryEmbeddingVectorStoreConfig):
    def transform_search_vector_store_request(  # noqa: PLR0913, PLR0917 - LiteLLM's signature
        self,
        vector_store_id: str,
        query: str | Sequence[str],
        vector_store_search_optional_params: VectorStoreSearchOptionalRequestParams,
        api_base: str,
        litellm_logging_obj: Logging,
        litellm_params: Mapping[str, object],
        extra_body: Mapping[str, object] | None = None,
        embedding_executor: object | None = None,
    ) -> tuple[str, dict[str, object]]:
        details = cast("dict[str, object]", litellm_logging_obj.model_call_details)
        previous = details.get("calls", 0)
        details["calls"] = (previous if isinstance(previous, int) else 0) + 1
        arguments: list[object] = [
            ["vector_store_id", vector_store_id],
            ["query", query],
            ["optional_params", dict(vector_store_search_optional_params)],
            ["api_base", api_base],
            ["litellm_params", dict(litellm_params)],
            ["extra_body", None if extra_body is None else dict(extra_body)],
        ]
        body = to_json(
            {
                "arguments": arguments,
                "embedding_executor": "none" if embedding_executor is None else "explicit",
            }
        )
        return (f"{api_base}/vector_stores/search", cast("dict[str, object]", body))


def concrete(base: type[object]) -> object:
    derived = type("Concrete", (base,), {})
    setattr(derived, "__abstractmethods__", frozenset())  # noqa: B010 - type has no typed attribute for it
    return derived()


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def as_object(value: Json) -> Object:
    return cast("Object", value)


def objects(value: Json) -> list[Object]:
    return cast("list[Object]", value)


def slot_value(slot: Object) -> Json:
    if slot.get("kind") == "value":
        return slot.get("value")
    pairs = cast("list[list[Json]]", slot.get("entries"))
    mapping: dict[object, Json] = {key: value for key, value in pairs}  # noqa: C416 - keys are not hashable-checked JSON
    return cast("Json", mapping)


def preparation_case(case: Object) -> Json:
    litellm_params: dict[str, object] = {}
    if "model" in case:
        litellm_params["litellm_embedding_model"] = case["model"]
    if "config" in case:
        litellm_params["litellm_embedding_config"] = slot_value(as_object(case["config"]))
    try:
        model = BaseQueryEmbeddingVectorStoreConfig.query_embedding_model(litellm_params)
        configuration = BaseQueryEmbeddingVectorStoreConfig.query_embedding_configuration(
            litellm_params
        )
    except (ValueError, ValidationError):
        return {"ok": False}
    return {"ok": True, "model": model, "configuration": to_json(dict(configuration))}


def query_preparation(fields: Object) -> Json:
    queries = cast("list[str | list[str]]", fields["queries"])
    texts = [BaseQueryEmbeddingVectorStoreConfig.query_text(query) for query in queries]
    cases = [preparation_case(case) for case in objects(fields["litellm_params"])]
    return to_json({"texts": texts, "cases": cases})


def precedence_case(case: Object) -> Json:
    explicit = LiteLLMVectorStoreEmbeddingExecutor() if case.get("explicit") else None
    router = cast("Router", object()) if case.get("router") else None
    metadata = as_object(case.get("metadata", {}))
    chosen = BaseQueryEmbeddingVectorStoreConfig.query_embedding_executor(
        explicit, router, metadata
    )
    if explicit is not None and chosen is explicit:
        return {"kind": "explicit"}
    if isinstance(chosen, RouterVectorStoreEmbeddingExecutor):
        return {"kind": "router", "metadata": to_json(dict(chosen.metadata))}
    return {"kind": "direct"}


def forward(fields: Object, *, asynchronous: bool) -> Json:
    config = cast("BaseQueryEmbeddingVectorStoreConfig", concrete(Fixture))
    logging = recording()
    extra = fields.get("extra_body")
    vector_store_id = cast("str", fields["vector_store_id"])
    query = cast("str | list[str]", fields["query"])
    optional_params = cast(
        "VectorStoreSearchOptionalRequestParams", as_object(fields["optional_params"])
    )
    api_base = cast("str", fields["api_base"])
    litellm_params = as_object(fields["litellm_params"])
    extra_body = None if extra is None else as_object(extra)
    if asynchronous:
        url, body = asyncio.run(
            config.atransform_search_vector_store_request(
                vector_store_id,
                query,
                optional_params,
                api_base,
                logging,
                litellm_params,
                extra_body,
            )
        )
    else:
        url, body = config.transform_search_vector_store_request(
            vector_store_id,
            query,
            optional_params,
            api_base,
            logging,
            litellm_params,
            extra_body,
        )
    calls = cast("dict[str, object]", logging.model_call_details).get("calls")
    return to_json({"url": url, "body": body, "calls": calls})


def url_outcome(config: BaseDirectVectorStoreConfig, api_base: str | None) -> Json:
    return to_json(config.get_complete_url(api_base=api_base, litellm_params={}))


def raised(call: Callable[[], object]) -> str:
    try:
        call()
    except NotImplementedError as error:
        return str(error)
    return "no error"


def defaults_report(fields: Object) -> Json:
    config = cast("BaseDirectVectorStoreConfig", concrete(BaseDirectVectorStoreConfig))
    litellm_params = as_object(fields.get("litellm_params", {}))
    urls = [
        url_outcome(config, api_base) for api_base in cast("list[str | None]", fields["api_bases"])
    ]
    return to_json(
        {
            "urls": urls,
            "credentials": config.get_auth_credentials(litellm_params),
            "endpoints": config.get_vector_store_endpoints_by_type(),
            "search_request": raised(
                lambda: config.transform_search_vector_store_request(
                    vector_store_id="vs_a",
                    query="hello",
                    vector_store_search_optional_params={},
                    api_base="",
                    litellm_logging_obj=recording(),
                    litellm_params=litellm_params,
                )
            ),
            "search_response": raised(
                lambda: config.transform_search_vector_store_response(
                    cast("httpx.Response", {}), recording()
                )
            ),
            "create_request": raised(lambda: config.transform_create_vector_store_request({}, "")),
            "create_response": raised(
                lambda: config.transform_create_vector_store_response(cast("httpx.Response", {}))
            ),
        }
    )


def run(fields: Object) -> Json:
    operation = fields.get("operation")
    if operation == "query_preparation":
        return query_preparation(fields)
    if operation == "executor_precedence":
        cases = [precedence_case(case) for case in objects(fields["cases"])]
        return to_json(
            {
                "cases": cases,
                "sync": forward(as_object(fields["forward"]), asynchronous=False),
                "async": forward(as_object(fields["forward"]), asynchronous=True),
            }
        )
    if operation == "direct_store_defaults":
        return defaults_report(fields)
    raise ValueError(f"unknown operation: {operation}")
