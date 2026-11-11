import os

os.environ["LITELLM_RUST"] = "false"

import json
from collections.abc import Callable, Mapping
from typing import TYPE_CHECKING, Final, TypeAlias, cast
from unittest import mock

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.vector_stores.transformation import BedrockVectorStoreConfig
from pydantic import BaseModel, ConfigDict, Field
from typing_extensions import TypeAliasType

litellm.suppress_debug_info = True

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
    from litellm.types.vector_stores import VectorStoreSearchOptionalRequestParams

Json = TypeAliasType("Json", None | bool | int | float | str | list["Json"] | dict[str, "Json"])
ENDPOINT: Final = "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases"


class UrlScenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    api_base: str | None = None
    litellm_params: dict[str, Json] = Field(default_factory=dict)
    environment: dict[str, str] = Field(default_factory=dict)


class ResponseScenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status: int
    headers: list[tuple[str, str]] = Field(default_factory=list)
    body: str
    query: str = ""


class OptionScenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    non_default_params: dict[str, Json] = Field(default_factory=dict)
    optional_params: dict[str, Json] = Field(default_factory=dict)


class ContextScenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    extra_body: Json | None = None
    litellm_params: dict[str, Json] = Field(default_factory=dict)


class RequestScenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    vector_store_id: str
    query: Json
    optional_params: dict[str, Json] = Field(default_factory=dict)
    api_base: str
    litellm_params: dict[str, Json] = Field(default_factory=dict)
    extra_body: Json | None = None


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    operation: str
    operators: list[str] = Field(default_factory=list)
    filters: list[Json] = Field(default_factory=list)
    urls: list[UrlScenario] = Field(default_factory=list)
    headers: list[tuple[str, str | None]] = Field(default_factory=list)
    metadata: list[dict[str, Json] | None] = Field(default_factory=list)
    responses: list[ResponseScenario] = Field(default_factory=list)
    options: list[OptionScenario] = Field(default_factory=list)
    contexts: list[ContextScenario] = Field(default_factory=list)
    requests: list[RequestScenario] = Field(default_factory=list)


class Logging:
    def __init__(self, query: str | None = None) -> None:
        self.model_call_details: dict[str, object] = (
            {} if query is None else {"query": query}
        )


def to_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def mapped_filters(
    filters: list[Json],
    mapper: Callable[[dict[str, object]], object],
) -> list[Json]:
    mapped: list[Json] = []
    for value in filters:
        try:
            result = mapper(cast(dict[str, object], value))
        except (KeyError, TypeError):
            mapped.append({"failed": True})
            continue
        mapped.append({"value": to_json(result)})
    return mapped


def filter_operator_map(config: BedrockVectorStoreConfig, params: Input) -> Json:
    operators = [
        config._map_operator_to_aws(operator)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        for operator in params.operators
    ]
    mapper = cast(
        Callable[[dict[str, object]], object],
        config._map_operator_filter,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return {"operators": cast(Json, operators), "filters": mapped_filters(params.filters, mapper)}


def nested_and_or_filters(config: BedrockVectorStoreConfig, params: Input) -> Json:
    mapper = cast(
        Callable[[dict[str, object]], object],
        config._map_and_or_filters,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return {"filters": mapped_filters(params.filters, mapper)}


def url_outcome(config: BedrockVectorStoreConfig, scenario: UrlScenario) -> Json:
    with mock.patch.dict(os.environ, scenario.environment, clear=True):
        try:
            url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                api_base=scenario.api_base,
                litellm_params=cast(dict[str, object], scenario.litellm_params),
            )
        except ValueError:
            return {"failed": True}
    return {"url": url}


def headers_outcome(config: BedrockVectorStoreConfig, params: Input) -> Json:
    resolved = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        headers=cast(dict[str, object], dict(params.headers)), litellm_params=None
    )
    return to_json(cast(object, resolved))


def knowledge_base_retrieve_url(config: BedrockVectorStoreConfig, params: Input) -> Json:
    urls = [url_outcome(config, scenario) for scenario in params.urls]
    return {
        "endpoints": to_json(config.get_vector_store_endpoints_by_type()),
        "credentials": to_json(
            config.get_auth_credentials(  # pyright: ignore[reportUnknownMemberType]
                litellm_params={}
            )
        ),
        "supported": to_json(config.get_supported_openai_params(model="")),
        "headers": headers_outcome(config, params),
        "urls": urls,
    }


def user_context_outcome(_config: BedrockVectorStoreConfig, params: Input) -> Json:
    selected: list[Json] = []
    for scenario in params.contexts:
        extra_body = cast(Mapping[str, object] | None, scenario.extra_body)
        value = BedrockVectorStoreConfig._user_context(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            extra_body=extra_body, litellm_params=scenario.litellm_params
        )
        selected.append(cast(Json, value))
    return {"selected": selected}


def location_file_id_name(config: BedrockVectorStoreConfig, params: Input) -> Json:
    values: list[Json] = []
    for metadata in params.metadata:
        resolved = cast(dict[str, object], metadata or {})
        values.append(
            {
                "file_id": cast(
                    Json,
                    config._get_file_id_from_metadata(resolved),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                ),
                "filename": cast(
                    Json,
                    config._get_filename_from_metadata(resolved),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                ),
                "attributes": cast(
                    Json,
                    config._get_attributes_from_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                        resolved
                    ),
                ),
            }
        )
    return {"values": values}


def decode_outcome(
    config: BedrockVectorStoreConfig, scenario: ResponseScenario
) -> tuple[Json, bool]:
    response = httpx.Response(
        scenario.status,
        content=scenario.body.encode(),
        headers=dict(scenario.headers),
    )
    logging_obj = Logging(scenario.query)
    try:
        result = config.transform_search_vector_store_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            response=response,
            litellm_logging_obj=cast("LiteLLMLoggingObj", logging_obj),
        )
    except BedrockError as error:
        return {"error": {"status_code": error.status_code}}, False
    return to_json(result), True


def decode_responses(
    config: BedrockVectorStoreConfig, params: Input, *, summary: bool
) -> Json:
    responses: list[Json] = []
    for scenario in params.responses:
        value, decoded = decode_outcome(config, scenario)
        if not decoded:
            responses.append(value)
        elif summary:
            data = cast(dict[str, Json], value).get("data") or []
            responses.append({"ok": True, "count": len(cast(list[Json], data))})
        else:
            responses.append({"value": value})
    return {"responses": responses}


def unsupported_create(config: BedrockVectorStoreConfig, _params: Input) -> Json:
    request_failed = False
    try:
        config.transform_create_vector_store_request(  # pyright: ignore[reportUnknownMemberType]
            vector_store_create_optional_params={}, api_base=ENDPOINT
        )
    except NotImplementedError:
        request_failed = True
    response_failed = False
    try:
        config.transform_create_vector_store_response(  # pyright: ignore[reportUnknownMemberType]
            response=httpx.Response(200)
        )
    except NotImplementedError:
        response_failed = True
    endpoints = config.get_vector_store_endpoints_by_type()
    return {
        "request": {"failed": request_failed},
        "response": {"failed": response_failed},
        "write": to_json(endpoints["write"]),
    }


def option_mapping(config: BedrockVectorStoreConfig, params: Input) -> Json:
    results: list[Json] = []
    for scenario in params.options:
        optional_params = dict(scenario.optional_params)
        try:
            mapped = config.map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                non_default_params=scenario.non_default_params,
                optional_params=optional_params,
                drop_params=False,
            )
        except (KeyError, TypeError):
            results.append({"failed": True})
            continue
        results.append({"params": to_json(cast(object, mapped))})
    return {"results": results}


def request_outcome(config: BedrockVectorStoreConfig, scenario: RequestScenario) -> Json:
    logging_obj = Logging()
    try:
        url, body = config.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            vector_store_id=scenario.vector_store_id,
            query=cast("str | list[str]", scenario.query),
            vector_store_search_optional_params=cast(
                "VectorStoreSearchOptionalRequestParams", scenario.optional_params
            ),
            api_base=scenario.api_base,
            litellm_logging_obj=cast("LiteLLMLoggingObj", logging_obj),
            litellm_params=scenario.litellm_params,
            extra_body=cast("dict[str, object] | None", scenario.extra_body),
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"failed": True}
    return {
        "url": url,
        "body": to_json(cast(object, body)),
        "query": cast(Json, logging_obj.model_call_details.get("query")),
    }


def retrieval_configuration(config: BedrockVectorStoreConfig, params: Input) -> Json:
    calls = [request_outcome(config, scenario) for scenario in params.requests]
    return {"calls": calls}


Handler: TypeAlias = Callable[[BedrockVectorStoreConfig, Input], Json]

HANDLERS: Final[dict[str, Handler]] = {
    "filter_operator_map": filter_operator_map,
    "nested_and_or_filters": nested_and_or_filters,
    "knowledge_base_retrieve_url": knowledge_base_retrieve_url,
    "user_context": user_context_outcome,
    "location_file_id_name": location_file_id_name,
    "attributes_and_scores": lambda config, params: decode_responses(
        config, params, summary=False
    ),
    "unsupported_create": unsupported_create,
    "option_mapping": option_mapping,
    "retrieval_configuration": retrieval_configuration,
    "knowledge_base_response_error": lambda config, params: decode_responses(
        config, params, summary=True
    ),
}


def run(input_value: Json) -> Json:
    supplied = Input.model_validate(cast(dict[str, object], input_value))
    handler = HANDLERS.get(supplied.operation)
    if handler is None:
        raise ValueError(f"unknown operation {supplied.operation}")
    return handler(BedrockVectorStoreConfig(), supplied)
