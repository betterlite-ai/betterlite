from typing import Final, NotRequired, TypedDict, cast

import litellm
from litellm.proxy import proxy_server
from litellm.router import Router
from litellm.types.utils import ModelInfo
from pydantic import JsonValue, TypeAdapter

MODELS: Final[TypeAdapter[list[dict[str, JsonValue]]]] = TypeAdapter(
    list[dict[str, JsonValue]]
)
ANSWER: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class CatalogEntry(TypedDict):
    model: str
    info: dict[str, JsonValue]
    provider: NotRequired[str | None]


class RouterRecord(TypedDict):
    async_client: NotRequired[str | None]
    discovered: NotRequired[dict[str, JsonValue]]


class Input(TypedDict):
    function: str
    model: NotRequired[dict[str, JsonValue] | None]
    debug: NotRequired[bool | None]
    router: NotRequired[RouterRecord | None]
    catalog: NotRequired[list[CatalogEntry]]
    cost_map: NotRequired[dict[str, JsonValue]]
    models: NotRequired[list[dict[str, JsonValue]]]
    sort_by: NotRequired[str | None]
    sort_order: NotRequired[str | None]
    access_group: NotRequired[str | None]
    exclude_auto_routers: NotRequired[bool | None]
    wildcard_only: NotRequired[bool | None]
    page: NotRequired[int | None]
    size: NotRequired[int | None]
    total_count: NotRequired[int | None]
    search: NotRequired[str | None]
    value: NotRequired[JsonValue]


class CaseCatalog:
    def __init__(self, entries: list[CatalogEntry]) -> None:
        self.entries = entries

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> ModelInfo:
        del api_base, api_key
        for entry in self.entries:
            provider = entry.get("provider")
            if entry["model"] == model and provider == custom_llm_provider:
                return cast(ModelInfo, entry["info"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L6416-L6421
        raise ValueError(f"the case catalog has no {model} entry")


class CaseRouter:
    def __init__(self, record: RouterRecord) -> None:
        discovered = record.get("discovered")
        if discovered is None:
            discovered = {}
        self.async_client = record.get("async_client")
        self.discovered = discovered

    def _get_client(
        self,
        deployment: dict[str, JsonValue],
        kwargs: dict[str, JsonValue],
        client_type: str,
    ) -> str | None:
        del deployment, kwargs, client_type
        return self.async_client

    def get_discovered_model_info(
        self, model_id: str | None
    ) -> dict[str, JsonValue] | None:
        del model_id
        return self.discovered


def model_list(input_value: Input) -> list[dict[str, JsonValue]]:
    models = input_value.get("models")
    if models is None:
        return []
    return models


def llm_router(record: RouterRecord | None) -> Router | None:
    if record is None:
        return None
    return cast(Router, CaseRouter(record))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L12293


def enrich_answer(input_value: Input) -> JsonValue:
    model = input_value.get("model")
    if model is None:
        raise ValueError("the enrich function needs a model")
    cost_map = input_value.get("cost_map")
    if cost_map is None:
        cost_map = {}
    entries = input_value.get("catalog")
    if entries is None:
        entries = []
    litellm.model_cost = cost_map
    litellm.get_model_info = CaseCatalog(entries)
    debug = input_value.get("debug")
    enriched = proxy_server._enrich_model_info_with_litellm_data(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14540-L14610
        model, debug is True, llm_router(input_value.get("router"))
    )
    return ANSWER.validate_python(enriched)


def sort_answer(input_value: Input) -> JsonValue:
    order = input_value.get("sort_order")
    if order is None:
        order = "asc"
    sorted_models = proxy_server._sort_models(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14897-L14983
        model_list(input_value), input_value.get("sort_by"), order
    )
    return MODELS.validate_python(sorted_models)


def auto_router_answer(input_value: Input) -> JsonValue:
    found: list[JsonValue] = [
        proxy_server._is_auto_router_model(model)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14986-L14999
        for model in model_list(input_value)
    ]
    return found


def access_group_answer(input_value: Input) -> JsonValue:
    group = input_value.get("access_group")
    if group is None:
        raise ValueError("the model_in_access_group function needs an access_group")
    found: list[JsonValue] = [
        proxy_server._model_in_access_group(model, group)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L15002-L15007
        for model in model_list(input_value)
    ]
    return found


def filters_answer(input_value: Input) -> JsonValue:
    found: list[JsonValue] = [
        proxy_server._matches_model_info_filters(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L15010-L15020
            model,
            input_value.get("exclude_auto_routers"),
            input_value.get("access_group"),
            input_value.get("wildcard_only"),
        )
        for model in model_list(input_value)
    ]
    return found


def paginate_answer(input_value: Input) -> JsonValue:
    page = input_value.get("page")
    size = input_value.get("size")
    if page is None or size is None:
        raise ValueError("the paginate function needs a page and a size")
    paginated = proxy_server._paginate_models_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L15023-L15065
        model_list(input_value),
        page,
        size,
        input_value.get("total_count"),
        input_value.get("search"),
    )
    return ANSWER.validate_python(paginated)


def datetime_answer(input_value: Input) -> JsonValue:
    normalized = proxy_server._normalize_datetime_for_sorting(input_value.get("value"))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L14855-L14894
    if normalized is None:
        return None
    return normalized.isoformat()


def answer(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "enrich":
        return enrich_answer(input_value)
    if function == "sort":
        return sort_answer(input_value)
    if function == "is_auto_router":
        return auto_router_answer(input_value)
    if function == "model_in_access_group":
        return access_group_answer(input_value)
    if function == "matches_filters":
        return filters_answer(input_value)
    if function == "paginate":
        return paginate_answer(input_value)
    if function == "datetime":
        return datetime_answer(input_value)
    raise ValueError(f"unsupported model_info_search function {function}")


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    litellm.suppress_debug_info = True
    return answer(TypeAdapter(Input).validate_python(input_value))
