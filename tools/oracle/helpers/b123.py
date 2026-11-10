from collections.abc import Callable
from typing import Literal, TypeAlias, TypedDict, cast
from unittest.mock import patch

from fastapi import FastAPI
from litellm.proxy import proxy_server
from litellm.proxy.common_utils.custom_openapi_spec import CustomOpenAPISpec
from litellm.proxy.common_utils.swagger_utils import ERROR_RESPONSES  # pyright: ignore[reportUnknownVariableType]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

LlmSchemaKey: TypeAlias = Literal["chat_completion", "embedding", "responses"]


class ErrorResponseTable(TypedDict):
    op: Literal["error_response_table"]


class ErrorResponseSchema(TypedDict):
    op: Literal["error_response_schema"]


class StableOperationId(TypedDict):
    op: Literal["stable_operation_id"]
    name: str
    path_format: str
    methods: list[str]


class EnsureUniqueOperationIds(TypedDict):
    op: Literal["ensure_unique_operation_ids"]
    document: dict[str, Json]
    reserved: list[str] | None


class QueryParamInput(TypedDict):
    name: str
    required: bool


class WebsocketRouteInput(TypedDict):
    path: str
    name: str | None
    query_params: list[QueryParamInput]


class InjectWebsocketStubs(TypedDict):
    op: Literal["inject_websocket_stubs"]
    document: dict[str, Json]
    routes: list[WebsocketRouteInput]


class LlmSchemas(TypedDict):
    chat_completion: dict[str, Json] | None
    embedding: dict[str, Json] | None
    responses: dict[str, Json] | None


class AddLlmRequestBodies(TypedDict):
    op: Literal["add_llm_request_bodies"]
    document: dict[str, Json]
    schemas: LlmSchemas


class RouteStub:
    def __init__(self, name: str, path_format: str, methods: set[str]) -> None:
        self.name = name
        self.path_format = path_format
        self.methods = methods


class QueryParamStub:
    def __init__(self, name: str, *, required: bool) -> None:
        self.name = name
        self.required = required


class DependantStub:
    def __init__(self, query_params: list[QueryParamStub]) -> None:
        self.query_params = query_params


class WebsocketRouteStub:
    def __init__(self, route: WebsocketRouteInput) -> None:
        self.path = route["path"]
        self.name = route["name"]
        self.dependant = DependantStub(
            [
                QueryParamStub(param["name"], required=param["required"])
                for param in route["query_params"]
            ]
        )


LLM_BODIES: tuple[tuple[str, list[str], LlmSchemaKey, str], ...] = (
    (
        "ProxyChatCompletionRequest",
        CustomOpenAPISpec.CHAT_COMPLETION_PATHS,
        "chat_completion",
        "chat completion",
    ),
    ("EmbeddingRequest", CustomOpenAPISpec.EMBEDDING_PATHS, "embedding", "embedding"),
    (
        "ResponsesAPIRequestParams",
        CustomOpenAPISpec.RESPONSES_API_PATHS,
        "responses",
        "responses API",
    ),
)


def _empty_response() -> dict[str, Json]:
    return {}


def error_response_component() -> Json:
    app = FastAPI(title="schema", version="0.0.0")
    app.add_api_route(
        "/x",
        _empty_response,
        methods=["GET"],
        responses=ERROR_RESPONSES,  # pyright: ignore[reportUnknownArgumentType]
        response_model=None,
    )
    schema = app.openapi()
    return cast(Json, schema["components"]["schemas"]["ErrorResponse"])


def _error_response_table(_input_value: dict[str, Json]) -> Json:
    return cast(
        Json,
        [
            {"status": status, "description": cast(str, entry["description"])}
            for status, entry in ERROR_RESPONSES.items()  # pyright: ignore[reportUnknownVariableType]
        ],
    )


def _error_response_schema(_input_value: dict[str, Json]) -> Json:
    return error_response_component()


def _stable_operation_id(input_value: dict[str, Json]) -> Json:
    request = cast(StableOperationId, input_value)
    route = RouteStub(request["name"], request["path_format"], set(request["methods"]))
    return proxy_server._generate_stable_operation_id(route)  # noqa: SLF001  # pyright: ignore[reportArgumentType, reportPrivateUsage]


def _ensure_unique_operation_ids(input_value: dict[str, Json]) -> Json:
    request = cast(EnsureUniqueOperationIds, input_value)
    document = request["document"]
    reserved = request["reserved"]
    reserved_ids: set[str] | None = set(reserved) if reserved is not None else None
    proxy_server.ensure_unique_openapi_operation_ids(document, reserved_ids)  # pyright: ignore[reportPrivateUsage]
    return cast(
        Json,
        {
            "document": document,
            "reserved": sorted(reserved_ids) if reserved_ids is not None else None,
        },
    )


def _inject_websocket_stubs(input_value: dict[str, Json]) -> Json:
    request = cast(InjectWebsocketStubs, input_value)
    document = request["document"]
    routes = [WebsocketRouteStub(route) for route in request["routes"]]
    proxy_server._inject_websocket_stubs_into_openapi_schema(document, routes)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return document


def _add_llm_request_bodies(input_value: dict[str, Json]) -> Json:
    request = cast(AddLlmRequestBodies, input_value)
    document = request["document"]
    schemas = request["schemas"]
    for schema_name, paths, schema_key, operation_name in LLM_BODIES:
        with patch.object(
            CustomOpenAPISpec,
            "get_pydantic_schema",
            return_value=schemas[schema_key],
        ):
            document = cast(
                dict[str, Json],
                CustomOpenAPISpec.add_request_schema(
                    document, object, schema_name, paths, operation_name
                ),
            )
    return document


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "error_response_table": _error_response_table,
    "error_response_schema": _error_response_schema,
    "stable_operation_id": _stable_operation_id,
    "ensure_unique_operation_ids": _ensure_unique_operation_ids,
    "inject_websocket_stubs": _inject_websocket_stubs,
    "add_llm_request_bodies": _add_llm_request_bodies,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("op")
    handler = HANDLERS.get(operation) if isinstance(operation, str) else None
    if handler is None:
        raise ValueError(f"unknown op: {operation}")
    return handler(input_value)
