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

litellm.suppress_debug_info = True

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
    from litellm.types.vector_stores import VectorStoreSearchOptionalRequestParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
PAIR_LENGTH: Final = 2
ENDPOINT: Final = "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases"
Handler: TypeAlias = Callable[[BedrockVectorStoreConfig, dict[str, Json]], Json]


class Logging:
    def __init__(self, query: str | None = None) -> None:
        self.model_call_details: dict[str, object] = (
            {} if query is None else {"query": query}
        )


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(dict[str, Json], value)


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return cast(list[Json], value)


def text_of(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_text(value: Json, name: str) -> str | None:
    return None if value is None else text_of(value, name)


def integer_of(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def texts_of(params: dict[str, Json], name: str) -> list[str]:
    return [text_of(entry, name) for entry in objects_of(params, name)]


def objects_of(params: dict[str, Json], name: str) -> list[Json]:
    value = params.get(name)
    return [] if value is None else list_of(value, name)


def pairs_of(value: Json, name: str) -> dict[str, Json]:
    pairs: dict[str, Json] = {}
    for entry in list_of(value, name):
        item = list_of(entry, name)
        if len(item) != PAIR_LENGTH:
            raise ValueError(f"{name} entries must be [key, value]")
        pairs[text_of(item[0], name)] = item[1]
    return pairs


def to_json(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def mapped_filters(
    params: dict[str, Json],
    mapper: Callable[[dict[str, object]], object],
) -> list[Json]:
    filters: list[Json] = []
    for value in objects_of(params, "filters"):
        try:
            mapped = mapper(cast(dict[str, object], value))
        except (KeyError, TypeError):
            filters.append({"failed": True})
            continue
        filters.append({"value": to_json(mapped)})
    return filters


def filter_operator_map(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    operators = [
        config._map_operator_to_aws(operator)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        for operator in texts_of(params, "operators")
    ]
    mapper = cast(
        Callable[[dict[str, object]], object],
        config._map_operator_filter,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return {"operators": cast(Json, operators), "filters": mapped_filters(params, mapper)}


def nested_and_or_filters(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    mapper = cast(
        Callable[[dict[str, object]], object],
        config._map_and_or_filters,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return {"filters": mapped_filters(params, mapper)}


def url_outcome(config: BedrockVectorStoreConfig, scenario: Json) -> Json:
    fields = object_of(scenario, "url")
    litellm_params = object_of(fields.get("litellm_params"), "litellm_params")
    environment = cast(dict[str, str], fields.get("environment") or {})
    api_base = optional_text(fields.get("api_base"), "api_base")
    with mock.patch.dict(os.environ, environment, clear=True):
        try:
            url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
                api_base=api_base, litellm_params=cast(dict[str, object], litellm_params)
            )
        except ValueError:
            return {"failed": True}
    return {"url": url}


def headers_outcome(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    headers = pairs_of(params.get("headers") or [], "headers")
    resolved = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        headers=cast(dict[str, object], headers), litellm_params=None
    )
    return to_json(cast(object, resolved))


def knowledge_base_retrieve_url(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    urls = [url_outcome(config, scenario) for scenario in objects_of(params, "urls")]
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


def user_context_outcome(_config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    selected: list[Json] = []
    for scenario in objects_of(params, "contexts"):
        fields = object_of(scenario, "contexts entry")
        litellm_params = object_of(fields.get("litellm_params") or {}, "litellm_params")
        extra_body = fields.get("extra_body")
        value = BedrockVectorStoreConfig._user_context(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            extra_body=cast(Mapping[str, object] | None, extra_body),
            litellm_params=litellm_params,
        )
        selected.append(cast(Json, value))
    return {"selected": selected}


def location_file_id_name(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    values: list[Json] = []
    for entry in objects_of(params, "metadata"):
        metadata = {} if entry is None else object_of(entry, "metadata")
        values.append(
            {
                "file_id": cast(
                    Json,
                    config._get_file_id_from_metadata(metadata),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                ),
                "filename": cast(
                    Json,
                    config._get_filename_from_metadata(metadata),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                ),
                "attributes": cast(
                    Json,
                    config._get_attributes_from_metadata(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                        cast(dict[str, object], metadata)
                    ),
                ),
            }
        )
    return {"values": values}


def decode_outcome(config: BedrockVectorStoreConfig, scenario: Json) -> tuple[Json, bool]:
    fields = object_of(scenario, "response")
    status = integer_of(fields.get("status"), "status")
    headers = pairs_of(fields.get("headers") or [], "headers")
    body = text_of(fields.get("body"), "body")
    query = text_of(fields.get("query") or "", "query")
    response = httpx.Response(
        status,
        content=body.encode(),
        headers=cast("dict[str, str]", headers),
    )
    logging_obj = Logging(query)
    try:
        result = config.transform_search_vector_store_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            response=response,
            litellm_logging_obj=cast("LiteLLMLoggingObj", logging_obj),
        )
    except BedrockError as error:
        return {"error": {"status_code": error.status_code}}, False
    return to_json(result), True


def decode_values(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    responses: list[Json] = []
    for scenario in objects_of(params, "responses"):
        value, decoded = decode_outcome(config, scenario)
        responses.append({"value": value} if decoded else value)
    return {"responses": responses}


def decode_errors(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    responses: list[Json] = []
    for scenario in objects_of(params, "responses"):
        value, decoded = decode_outcome(config, scenario)
        if not decoded:
            responses.append(value)
            continue
        data = object_of(value, "value").get("data") or []
        responses.append({"ok": True, "count": len(cast(list[Json], data))})
    return {"responses": responses}


def unsupported_create(config: BedrockVectorStoreConfig, _params: dict[str, Json]) -> Json:
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


def option_mapping(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    results: list[Json] = []
    for scenario in objects_of(params, "options"):
        fields = object_of(scenario, "option")
        optional_params = dict(object_of(fields.get("optional_params") or {}, "optional_params"))
        non_default_params = dict(
            object_of(fields.get("non_default_params") or {}, "non_default_params")
        )
        try:
            mapped = config.map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
                non_default_params=non_default_params,
                optional_params=optional_params,
                drop_params=False,
            )
        except (KeyError, TypeError):
            results.append({"failed": True})
            continue
        results.append({"params": to_json(cast(object, mapped))})
    return {"results": results}


def request_outcome(config: BedrockVectorStoreConfig, scenario: Json) -> Json:
    fields = object_of(scenario, "request")
    vector_store_id = text_of(fields.get("vector_store_id"), "vector_store_id")
    optional_params = object_of(fields.get("optional_params") or {}, "optional_params")
    litellm_params = object_of(fields.get("litellm_params") or {}, "litellm_params")
    api_base = text_of(fields.get("api_base"), "api_base")
    extra_body = fields.get("extra_body")
    logging_obj = Logging()
    try:
        url, body = config.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
            vector_store_id=vector_store_id,
            query=cast("str | list[str]", fields.get("query")),
            vector_store_search_optional_params=cast(
                "VectorStoreSearchOptionalRequestParams", optional_params
            ),
            api_base=api_base,
            litellm_logging_obj=cast("LiteLLMLoggingObj", logging_obj),
            litellm_params=litellm_params,
            extra_body=cast("dict[str, object] | None", extra_body),
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"failed": True}
    return {
        "url": url,
        "body": to_json(cast(object, body)),
        "query": cast(Json, logging_obj.model_call_details.get("query")),
    }


def retrieval_configuration(config: BedrockVectorStoreConfig, params: dict[str, Json]) -> Json:
    calls = [request_outcome(config, scenario) for scenario in objects_of(params, "requests")]
    return {"calls": calls}


HANDLERS: Final[dict[str, Handler]] = {
    "filter_operator_map": filter_operator_map,
    "nested_and_or_filters": nested_and_or_filters,
    "knowledge_base_retrieve_url": knowledge_base_retrieve_url,
    "user_context": user_context_outcome,
    "location_file_id_name": location_file_id_name,
    "attributes_and_scores": decode_values,
    "unsupported_create": unsupported_create,
    "option_mapping": option_mapping,
    "retrieval_configuration": retrieval_configuration,
    "knowledge_base_response_error": decode_errors,
}


def run(input_value: Json) -> Json:
    params = object_of(input_value, "input")
    operation = text_of(params.get("operation"), "operation")
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation {operation}")
    return handler(BedrockVectorStoreConfig(), params)
