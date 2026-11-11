import os

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass, field
from typing import Final, Literal, TypeAlias

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.vector_stores.transformation import BedrockVectorStoreConfig
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Json: TypeAlias = JsonValue
Object: TypeAlias = dict[str, Json]
JSON: Final = TypeAdapter(Json)
ANY: Final = TypeAdapter(object)
ENDPOINT: Final = "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases"


@dataclass(frozen=True)
class UrlScenario:
    api_base: str | None = None
    litellm_params: Object = field(default_factory=dict)
    environment: dict[str, str] = field(default_factory=dict)


@dataclass(frozen=True)
class ResponseScenario:
    status: int
    body: str
    headers: list[tuple[str, str]] = field(default_factory=list)
    query: str = ""


@dataclass(frozen=True)
class OptionScenario:
    non_default_params: Object = field(default_factory=dict)
    optional_params: Object = field(default_factory=dict)


@dataclass(frozen=True)
class ContextScenario:
    extra_body: Object | None = None
    litellm_params: Object = field(default_factory=dict)


@dataclass(frozen=True)
class RequestScenario:
    vector_store_id: str
    query: str | list[str]
    api_base: str
    optional_params: Object = field(default_factory=dict)
    litellm_params: Object = field(default_factory=dict)
    extra_body: Object | None = None


@dataclass(frozen=True)
class Input:
    operation: Literal[
        "filter_operator_map",
        "nested_and_or_filters",
        "knowledge_base_retrieve_url",
        "user_context",
        "location_file_id_name",
        "attributes_and_scores",
        "unsupported_create",
        "option_mapping",
        "retrieval_configuration",
        "knowledge_base_response_error",
    ]
    operators: list[str] = field(default_factory=list)
    filters: list[Json] = field(default_factory=list)
    urls: list[UrlScenario] = field(default_factory=list)
    headers: list[tuple[str, str | None]] = field(default_factory=list)
    metadata: list[Object | None] = field(default_factory=list)
    responses: list[ResponseScenario] = field(default_factory=list)
    options: list[OptionScenario] = field(default_factory=list)
    contexts: list[ContextScenario] = field(default_factory=list)
    requests: list[RequestScenario] = field(default_factory=list)


@dataclass(frozen=True)
class SearchPage:
    data: list[Json]


@dataclass
class QueryLog:
    model_call_details: dict[str, str] = field(default_factory=dict)


@contextmanager
def environment(variables: dict[str, str]) -> Generator[None, None, None]:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(variables)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


def filter_outcome(config: BedrockVectorStoreConfig, value: Json, *, nested: bool) -> Json:
    try:
        if nested:
            mapped = JSON.validate_python(config._map_and_or_filters(value))  # type: ignore[arg-type,misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L109
        else:
            mapped = JSON.validate_python(config._map_operator_filter(value))  # type: ignore[arg-type,misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L94
    except (KeyError, TypeError):
        return {"failed": True}
    return {"value": mapped}


def url_outcome(config: BedrockVectorStoreConfig, scenario: UrlScenario) -> Json:
    with environment(scenario.environment):
        try:
            url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L191
                api_base=scenario.api_base, litellm_params=scenario.litellm_params
            )
        except ValueError:
            return {"failed": True}
    return {"url": url}


def retrieve_url(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    headers: Object = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L186
        headers=dict(supplied.headers), litellm_params=None  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L186
    )
    return {
        "endpoints": JSON.validate_json(
            ANY.dump_json(config.get_vector_store_endpoints_by_type())
        ),
        "credentials": JSON.validate_python(
            config.get_auth_credentials(litellm_params={})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L52
        ),
        "supported": JSON.validate_python(config.get_supported_openai_params(model="")),
        "headers": headers,
        "urls": [url_outcome(config, scenario) for scenario in supplied.urls],
    }


def selected_user_contexts(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    return {
        "selected": [
            JSON.validate_python(
                config._user_context(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L255
                    extra_body=scenario.extra_body, litellm_params=scenario.litellm_params
                )
            )
            for scenario in supplied.contexts
        ]
    }


def location_values(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    values: list[Json] = []
    for entry in supplied.metadata:
        metadata: Object = {} if entry is None else entry
        values.append(
            {
                "file_id": config._get_file_id_from_metadata(metadata),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L287
                "filename": config._get_filename_from_metadata(metadata),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L299
                "attributes": JSON.validate_python(
                    config._get_attributes_from_metadata(metadata)  # type: ignore[arg-type]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L321
                ),
            }
        )
    return {"values": values}


def decoded_responses(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    summary = supplied.operation == "knowledge_base_response_error"
    responses: list[Json] = []
    for scenario in supplied.responses:
        response = httpx.Response(
            scenario.status, content=scenario.body.encode(), headers=scenario.headers
        )
        try:
            value = JSON.validate_python(
                config.transform_search_vector_store_response(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L330
                    response=response,
                    litellm_logging_obj=QueryLog({"query": scenario.query}),  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L331
                )
            )
        except BedrockError as error:
            responses.append({"error": {"status_code": error.status_code}})
        else:
            page = TypeAdapter(SearchPage).validate_python(value)
            responses.append({"ok": True, "count": len(page.data)} if summary else {"value": value})
    return {"responses": responses}


def unsupported_create(config: BedrockVectorStoreConfig) -> Json:
    request_failed = False
    try:
        config.transform_create_vector_store_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L370
            vector_store_create_optional_params={},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L370
            api_base=ENDPOINT,
        )
    except NotImplementedError:
        request_failed = True
    response_failed = False
    try:
        config.transform_create_vector_store_response(response=httpx.Response(200))
    except NotImplementedError:
        response_failed = True
    return {
        "request": {"failed": request_failed},
        "response": {"failed": response_failed},
        "write": JSON.validate_python(config.get_vector_store_endpoints_by_type()["write"]),
    }


def option_mapping(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    results: list[Json] = []
    for scenario in supplied.options:
        optional_params: Object = dict(scenario.optional_params)
        try:
            mapped: Object = config.map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L155
                non_default_params=scenario.non_default_params,
                optional_params=optional_params,
                drop_params=False,
            )
        except (KeyError, TypeError):
            results.append({"failed": True})
            continue
        results.append({"params": mapped})
    return {"results": results}


def retrieval_configuration(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    calls: list[Json] = []
    for scenario in supplied.requests:
        log = QueryLog()
        try:
            url, body = config.transform_search_vector_store_request(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L201
                vector_store_id=scenario.vector_store_id,
                query=scenario.query,
                vector_store_search_optional_params=scenario.optional_params,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L1
                api_base=scenario.api_base,
                litellm_logging_obj=log,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L207
                litellm_params=scenario.litellm_params,
                extra_body=scenario.extra_body,
            )
        except (AttributeError, KeyError, TypeError, ValueError):
            calls.append({"failed": True})
            continue
        calls.append({"url": url, "body": body, "query": log.model_call_details["query"]})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L210
    return {"calls": calls}


def filters_outcome(config: BedrockVectorStoreConfig, supplied: Input) -> Json:
    nested = supplied.operation == "nested_and_or_filters"
    outcome: Object = {
        "filters": [filter_outcome(config, value, nested=nested) for value in supplied.filters]
    }
    if not nested:
        outcome["operators"] = [
            config._map_operator_to_aws(operator)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/vector_stores/transformation.py#L64
            for operator in supplied.operators
        ]
    return outcome


Handler: TypeAlias = Callable[[BedrockVectorStoreConfig, Input], Json]
HANDLERS: Final[dict[str, Handler]] = {
    "filter_operator_map": filters_outcome,
    "nested_and_or_filters": filters_outcome,
    "knowledge_base_retrieve_url": retrieve_url,
    "user_context": selected_user_contexts,
    "location_file_id_name": location_values,
    "attributes_and_scores": decoded_responses,
    "knowledge_base_response_error": decoded_responses,
    "option_mapping": option_mapping,
    "retrieval_configuration": retrieval_configuration,
}


def run(input_value: Object) -> Json:
    supplied = TypeAdapter(Input).validate_python(input_value)
    config = BedrockVectorStoreConfig()
    if supplied.operation == "unsupported_create":
        return unsupported_create(config)
    return HANDLERS[supplied.operation](config, supplied)
