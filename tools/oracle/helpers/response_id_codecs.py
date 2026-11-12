import logging
from collections.abc import Callable
from types import TracebackType
from typing import Literal, NotRequired, Self, TypeAlias, TypedDict

from litellm.responses.utils import ResponsesAPIRequestUtils
from litellm.types.responses.main import DecodedResponseId
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ResponseBuild(TypedDict):
    custom_llm_provider: str | None
    model_id: str | None
    response_id: str


class ItemBuild(TypedDict):
    model_id: str
    item_id: str


class ContentBuild(TypedDict):
    encrypted_content: str
    model_id: str


class ContainerBuild(TypedDict):
    custom_llm_provider: str | None
    model_id: str | None
    container_id: str


class Input(TypedDict):
    operation: Literal[
        "response_roundtrip",
        "response_fallback",
        "encrypted_item_roundtrip",
        "content_wrapper",
        "container_delimiters",
    ]
    scenarios: NotRequired[list[ResponseBuild]]
    ids: NotRequired[list[str]]
    items: NotRequired[list[ItemBuild]]
    contents: NotRequired[list[ContentBuild]]
    containers: NotRequired[list[ContainerBuild]]


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.levels: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname)


class Diagnostics:
    def __init__(self) -> None:
        self.logger = logging.getLogger("LiteLLM")
        self.capture = Capture()
        self.previous_level = logging.NOTSET
        self.previous_handlers: list[logging.Handler] = []

    def __enter__(self) -> Self:
        self.previous_handlers = list(self.logger.handlers)
        for handler in self.previous_handlers:
            self.logger.removeHandler(handler)
        self.previous_level = self.logger.level
        self.logger.setLevel(logging.DEBUG)
        self.logger.addHandler(self.capture)
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.logger.removeHandler(self.capture)
        self.logger.setLevel(self.previous_level)
        for handler in self.previous_handlers:
            self.logger.addHandler(handler)

    def take(self) -> str | None:
        reported = "debug" if self.capture.levels else None
        self.capture.levels.clear()
        return reported


def _decoded(decoded: DecodedResponseId) -> dict[str, Json]:
    return {
        "custom_llm_provider": decoded.get("custom_llm_provider"),
        "model_id": decoded.get("model_id"),
        "response_id": decoded.get("response_id"),
    }


def _item(decoded: dict[str, str] | None) -> Json:
    if decoded is None:
        return None
    return {"model_id": decoded["model_id"], "item_id": decoded["item_id"]}


def _decode_item(encoded: str) -> Json:
    decoded = ResponsesAPIRequestUtils._decode_encrypted_item_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L395
        encoded
    )
    return _item(decoded)


def _decode_content(encoded: str) -> Json:
    model_id, content = ResponsesAPIRequestUtils._unwrap_encrypted_content_with_model_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L434
        encoded
    )
    return {"model_id": model_id, "content": content}


def _decode_container(encoded: str) -> Json:
    decoded = ResponsesAPIRequestUtils._decode_container_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L730
        encoded
    )
    return _decoded(decoded)


def _item_build(item: ItemBuild) -> Json:
    encoded = ResponsesAPIRequestUtils._build_encrypted_item_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L385
        item["model_id"], item["item_id"]
    )
    return {"encoded": encoded, "decoded": _decode_item(encoded)}


def _content_build(build: ContentBuild) -> Json:
    wrapped = ResponsesAPIRequestUtils._wrap_encrypted_content_with_model_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L421
        build["encrypted_content"], build["model_id"]
    )
    return {"wrapped": wrapped, "unwrapped": _decode_content(wrapped)}


def _container_build(container: ContainerBuild) -> Json:
    encoded = ResponsesAPIRequestUtils._build_container_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L713
        container["custom_llm_provider"], container["model_id"], container["container_id"]
    )
    return {"encoded": encoded, "decoded": _decode_container(encoded)}


def _decoded_results(ids: list[str], decode: Callable[[str], Json]) -> list[Json]:
    results: list[Json] = []
    for encoded in ids:
        with Diagnostics() as diagnostics:
            decoded = decode(encoded)
            diagnostic = diagnostics.take()
        results.append({"decoded": decoded, "diagnostic": diagnostic})
    return results


def _roundtrip(
    key: str, builds: list[Json], ids: list[str], decode: Callable[[str], Json]
) -> dict[str, Json]:
    return {
        key: builds,
        "ids": _decoded_results(ids, decode),
    }


def _required_ids(input_value: Input) -> list[str]:
    ids = input_value.get("ids")
    if ids is None:
        raise ValueError("the case needs ids")
    return ids


def _required_items(input_value: Input) -> list[ItemBuild]:
    items = input_value.get("items")
    if items is None:
        raise ValueError("the case needs items")
    return items


def _required_contents(input_value: Input) -> list[ContentBuild]:
    contents = input_value.get("contents")
    if contents is None:
        raise ValueError("the case needs contents")
    return contents


def _required_containers(input_value: Input) -> list[ContainerBuild]:
    containers = input_value.get("containers")
    if containers is None:
        raise ValueError("the case needs containers")
    return containers


def _response_roundtrip(input_value: Input) -> dict[str, Json]:
    scenarios = input_value.get("scenarios")
    if scenarios is None:
        raise ValueError("response_roundtrip needs scenarios")
    results: list[Json] = []
    for scenario in scenarios:
        encoded = ResponsesAPIRequestUtils._build_responses_api_response_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L611
            scenario["custom_llm_provider"], scenario["model_id"], scenario["response_id"]
        )
        previous_response_id = (
            ResponsesAPIRequestUtils.decode_previous_response_id_to_original_previous_response_id(
                encoded
            )
        )
        results.append(
            {
                "encoded": encoded,
                "decoded": _decoded(
                    ResponsesAPIRequestUtils._decode_responses_api_response_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L624
                        encoded
                    )
                ),
                "is_encoded": ResponsesAPIRequestUtils._is_litellm_encoded_response_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L677
                    encoded
                ),
                "previous_response_id": previous_response_id,
                "model_lookup": ResponsesAPIRequestUtils.get_model_id_from_response_id(encoded),
            }
        )
    return {
        "scenarios": results,
        "null_lookup": ResponsesAPIRequestUtils.get_model_id_from_response_id(None),
    }


def _response_fallback(input_value: Input) -> dict[str, Json]:
    ids = _required_ids(input_value)
    results: list[Json] = []
    for encoded in ids:
        with Diagnostics() as diagnostics:
            decoded = ResponsesAPIRequestUtils._decode_responses_api_response_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L624
                encoded
            )
            diagnostic = diagnostics.take()
        previous_response_id = (
            ResponsesAPIRequestUtils.decode_previous_response_id_to_original_previous_response_id(
                encoded
            )
        )
        results.append(
            {
                "decoded": _decoded(decoded),
                "is_encoded": ResponsesAPIRequestUtils._is_litellm_encoded_response_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L677
                    encoded
                ),
                "previous_response_id": previous_response_id,
                "diagnostic": diagnostic,
            }
        )
    return {"ids": results}


def _encrypted_item_roundtrip(input_value: Input) -> dict[str, Json]:
    items = _required_items(input_value)
    ids = _required_ids(input_value)
    builds = [_item_build(item) for item in items]
    return _roundtrip("items", builds, ids, _decode_item)


def _content_wrapper(input_value: Input) -> dict[str, Json]:
    contents = _required_contents(input_value)
    ids = _required_ids(input_value)
    builds = [_content_build(build) for build in contents]
    return _roundtrip("contents", builds, ids, _decode_content)


def _container_delimiters(input_value: Input) -> dict[str, Json]:
    containers = _required_containers(input_value)
    ids = _required_ids(input_value)
    builds = [_container_build(container) for container in containers]
    return _roundtrip("containers", builds, ids, _decode_container)


def run(input_value: Input) -> Json:
    operation: str = input_value["operation"]
    if operation == "response_roundtrip":
        return _response_roundtrip(input_value)
    if operation == "response_fallback":
        return _response_fallback(input_value)
    if operation == "encrypted_item_roundtrip":
        return _encrypted_item_roundtrip(input_value)
    if operation == "content_wrapper":
        return _content_wrapper(input_value)
    if operation == "container_delimiters":
        return _container_delimiters(input_value)
    raise ValueError(f"unsupported response_id_codecs operation: {operation}")
