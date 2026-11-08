import asyncio
import datetime as dt
import json
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.vector_store.transformation import (
    BaseVectorStoreConfig,
    vector_store_request_metadata,
)
from litellm.types.vector_stores import (
    VectorStoreSearchOptionalRequestParams,
    VectorStoreSearchResponse,
)
from litellm.vector_stores.utils import VectorStoreRequestUtils
from pydantic import ValidationError

if TYPE_CHECKING:
    from collections.abc import Mapping

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
SearchArguments: TypeAlias = tuple[
    str | list[str],
    VectorStoreSearchOptionalRequestParams,
    str,
    dict[str, object],
    dict[str, object] | None,
]


class Fixture(BaseVectorStoreConfig):
    def transform_search_vector_store_request(  # noqa: PLR0913, PLR0917 - LiteLLM's signature
        self,
        vector_store_id: str,
        query: str | list[str],
        vector_store_search_optional_params: VectorStoreSearchOptionalRequestParams,
        api_base: str,
        litellm_logging_obj: Logging,
        litellm_params: dict[str, object],
        extra_body: dict[str, object] | None = None,
    ) -> tuple[str, dict[str, object]]:
        details = cast("dict[str, object]", litellm_logging_obj.model_call_details)
        previous = details.get("calls", 0)
        calls = (previous if isinstance(previous, int) else 0) + 1
        details["calls"] = calls
        return (
            f"{api_base}/vector_stores/search",
            {
                "vector_store_id": vector_store_id,
                "query": query,
                "optional_params": dict(vector_store_search_optional_params),
                "api_base": api_base,
                "litellm_params": dict(litellm_params),
                "extra_body": None if extra_body is None else dict(extra_body),
            },
        )


class RecordingConfig(BaseVectorStoreConfig):
    def map_openai_params(
        self,
        non_default_params: dict[str, object],
        optional_params: dict[str, object],
        drop_params: bool,  # noqa: FBT001 - LiteLLM's signature
    ) -> dict[str, object]:
        return {
            **optional_params,
            "observed": {
                "non_default_params": dict(non_default_params),
                "drop_params": drop_params,
            },
        }


def fixture(config_type: type[BaseVectorStoreConfig]) -> BaseVectorStoreConfig:
    derived = cast(
        "type[BaseVectorStoreConfig]", type("FixtureConfig", (config_type,), {})
    )
    setattr(derived, "__abstractmethods__", frozenset())  # noqa: B010 - type has no typed attribute for it
    return derived()


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def as_object(value: Json) -> Object:
    return cast("Object", value)


def objects(value: Json) -> list[Object]:
    return cast("list[Object]", value)


def optional_filters(fields: Object) -> Json:
    create: list[Json] = [
        to_json(
            VectorStoreRequestUtils.get_requested_vector_store_create_optional_param(
                params
            )
        )
        for params in objects(fields["create"])
    ]
    search: list[Json] = [
        {
            "default": to_json(
                VectorStoreRequestUtils.get_requested_vector_store_search_optional_param(
                    params, fixture(BaseVectorStoreConfig)
                )
            ),
            "mapped": to_json(
                VectorStoreRequestUtils.get_requested_vector_store_search_optional_param(
                    params, fixture(RecordingConfig)
                )
            ),
        }
        for params in objects(fields["search"])
    ]
    return to_json({"create": create, "search": search})


def slot_value(slot: Json) -> Json:
    described = as_object(slot)
    kind = described.get("kind")
    if kind == "value":
        return described.get("value")
    if kind == "entries":
        pairs = cast("list[list[Json]]", described.get("entries"))
        entries: dict[object, Json] = {}
        for entry in pairs:
            entries[entry[0]] = entry[1]
        return cast("Json", entries)
    raise ValueError(f"unknown slot kind: {kind}")


def metadata_kwargs(case: Object) -> dict[object, object]:
    kwargs: dict[object, object] = {}
    for name in ("litellm_metadata", "metadata"):
        if name in case:
            kwargs[name] = slot_value(case[name])
    return kwargs


def metadata_case(case: Object) -> Json:
    kwargs = metadata_kwargs(case)
    try:
        result = vector_store_request_metadata(cast("Mapping[str, object]", kwargs))
    except ValidationError:
        return {"ok": False, "class": "ValidationError"}
    return {
        "ok": True,
        "kwargs": to_json(kwargs),
        "result": to_json(dict(result)),
    }


def metadata_cases(fields: Object) -> Json:
    return cast("Json", [metadata_case(case) for case in objects(fields["cases"])])


def api_base_outcome(config: BaseVectorStoreConfig, api_base: str | None) -> Json:
    try:
        return config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]
            api_base=api_base, litellm_params={}
        )
    except ValueError as error:
        return {"error": {"class": type(error).__name__, "message": str(error)}}


def error_facts(config: BaseVectorStoreConfig, error: Object) -> Json:
    headers = cast("dict[str, object]", as_object(error["headers"]))
    try:
        config.get_error_class(  # pyright: ignore[reportUnknownMemberType]
            error_message=cast("str", error["message"]),
            status_code=cast("int", error["status_code"]),
            headers=headers,
        )
    except BaseLLMException as raised:
        raised_headers = cast("object", raised.headers)  # pyright: ignore[reportUnknownMemberType]
        return {
            "status_code": raised.status_code,
            "message": raised.message,
            "headers": to_json(raised_headers),
        }
    raise AssertionError("get_error_class returned without raising")


def search_arguments(fields: Object) -> SearchArguments:
    extra = fields.get("extra_body")
    return (
        cast("str | list[str]", fields["query"]),
        cast(
            "VectorStoreSearchOptionalRequestParams",
            as_object(fields["optional_params"]),
        ),
        cast("str", fields["api_base"]),
        cast("dict[str, object]", as_object(fields["litellm_params"])),
        None if extra is None else cast("dict[str, object]", as_object(extra)),
    )


def sync_search(
    config: BaseVectorStoreConfig, fields: Object, logging: Logging
) -> tuple[str, dict[str, object]]:
    query, optional, api_base, litellm_params, extra_body = search_arguments(fields)
    return cast(
        "tuple[str, dict[str, object]]",
        config.transform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType]
            vector_store_id=cast("str", fields["vector_store_id"]),
            query=query,
            vector_store_search_optional_params=optional,
            api_base=api_base,
            litellm_logging_obj=logging,
            litellm_params=litellm_params,
            extra_body=extra_body,
        ),
    )


async def async_search(
    config: BaseVectorStoreConfig, fields: Object, logging: Logging
) -> tuple[str, dict[str, object]]:
    query, optional, api_base, litellm_params, extra_body = search_arguments(fields)
    return cast(
        "tuple[str, dict[str, object]]",
        await config.atransform_search_vector_store_request(  # pyright: ignore[reportUnknownMemberType]
            vector_store_id=cast("str", fields["vector_store_id"]),
            query=query,
            vector_store_search_optional_params=optional,
            api_base=api_base,
            litellm_logging_obj=logging,
            litellm_params=litellm_params,
            extra_body=extra_body,
        ),
    )


def forward(config: BaseVectorStoreConfig, fields: Object, *, asynchronous: bool) -> Json:
    logging = Logging(
        model="text-embedding-3-small",
        messages=None,
        stream=False,
        call_type="embedding",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="vector-store-contracts",
        function_id="vector-store-contracts",
    )
    outcome = (
        asyncio.run(async_search(config, fields, logging))
        if asynchronous
        else sync_search(config, fields, logging)
    )
    url, body = outcome
    calls = cast("dict[str, object]", logging.model_call_details).get("calls")
    return to_json({"url": url, "body": body, "logging": {"calls": calls}})


def defaults_report(fields: Object) -> Json:
    config = fixture(Fixture)
    optional = cast("dict[str, object]", as_object(fields["optional_params"]))
    headers = cast("dict[str, str]", fields["headers"])
    signed = cast(
        "tuple[dict[str, object], bytes | None]",
        config.sign_request(  # pyright: ignore[reportUnknownMemberType]
            headers=headers,
            optional_params=optional,
            request_data=cast("dict[str, object]", as_object(fields["request_data"])),
            api_base=cast("str", fields["api_base"]),
            api_key=cast("str | None", fields["api_key"]),
        ),
    )
    signed_headers, body = signed
    urls: list[Json] = [
        api_base_outcome(config, api_base)
        for api_base in cast("list[str | None]", fields["api_bases"])
    ]
    prompt_cost, completion_cost = config.calculate_vector_store_cost(
        cast("VectorStoreSearchResponse", {})
    )
    forward_fields = as_object(fields["forward"])
    return to_json(
        {
            "supported": list(
                config.get_supported_openai_params("openai/text-embedding-3-small")
            ),
            "mapped": dict(
                cast(
                    "dict[str, object]",
                    config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
                        non_default_params=cast(
                            "dict[str, object]",
                            as_object(fields["non_default_params"]),
                        ),
                        optional_params=optional,
                        drop_params=False,
                    ),
                )
            ),
            "environment": cast(
                "dict[str, object]",
                config.validate_environment(  # pyright: ignore[reportUnknownMemberType]
                    headers=headers, litellm_params=None
                ),
            ),
            "urls": urls,
            "signed_headers": signed_headers,
            "signed_body": None if body is None else list(body),
            "cost": [prompt_cost, completion_cost],
            "error": error_facts(config, as_object(fields["error"])),
            "sync": forward(config, forward_fields, asynchronous=False),
            "async": forward(config, forward_fields, asynchronous=True),
        }
    )


def run(fields: Object) -> Json:
    operation = fields.get("operation")
    if operation == "optional_parameter_filters":
        return optional_filters(fields)
    if operation == "request_metadata_precedence":
        return metadata_cases(fields)
    if operation == "base_contract_defaults":
        return defaults_report(fields)
    raise ValueError(f"unknown operation: {operation}")
