import logging
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


def _item(decoded: dict[str, str] | None) -> dict[str, Json] | None:
    if decoded is None:
        return None
    return {"model_id": decoded["model_id"], "item_id": decoded["item_id"]}


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
    ids = input_value.get("ids")
    if ids is None:
        raise ValueError("response_fallback needs ids")
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
    items = input_value.get("items")
    ids = input_value.get("ids")
    if items is None:
        raise ValueError("encrypted_item_roundtrip needs items")
    if ids is None:
        raise ValueError("encrypted_item_roundtrip needs ids")
    builds: list[Json] = []
    for item in items:
        encoded = ResponsesAPIRequestUtils._build_encrypted_item_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L385
            item["model_id"], item["item_id"]
        )
        builds.append(
            {
                "encoded": encoded,
                "decoded": _item(
                    ResponsesAPIRequestUtils._decode_encrypted_item_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L395
                        encoded
                    )
                ),
            }
        )
    decodes: list[Json] = []
    for encoded in ids:
        with Diagnostics() as diagnostics:
            item_decoded = ResponsesAPIRequestUtils._decode_encrypted_item_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L395
                encoded
            )
            diagnostic = diagnostics.take()
        decodes.append({"decoded": _item(item_decoded), "diagnostic": diagnostic})
    return {"items": builds, "ids": decodes}


def _content_wrapper(input_value: Input) -> dict[str, Json]:
    contents = input_value.get("contents")
    ids = input_value.get("ids")
    if contents is None:
        raise ValueError("content_wrapper needs contents")
    if ids is None:
        raise ValueError("content_wrapper needs ids")
    builds: list[Json] = []
    for build in contents:
        wrapped = ResponsesAPIRequestUtils._wrap_encrypted_content_with_model_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L421
            build["encrypted_content"], build["model_id"]
        )
        model_id, original = ResponsesAPIRequestUtils._unwrap_encrypted_content_with_model_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L434
            wrapped
        )
        builds.append(
            {"wrapped": wrapped, "unwrapped": {"model_id": model_id, "content": original}}
        )
    decodes: list[Json] = []
    for encoded in ids:
        with Diagnostics() as diagnostics:
            model_id, content = ResponsesAPIRequestUtils._unwrap_encrypted_content_with_model_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L434
                encoded
            )
            diagnostic = diagnostics.take()
        decodes.append(
            {
                "model_id": model_id,
                "content": content,
                "diagnostic": diagnostic,
            }
        )
    return {"contents": builds, "ids": decodes}


def _container_delimiters(input_value: Input) -> dict[str, Json]:
    containers = input_value.get("containers")
    ids = input_value.get("ids")
    if containers is None:
        raise ValueError("container_delimiters needs containers")
    if ids is None:
        raise ValueError("container_delimiters needs ids")
    builds: list[Json] = []
    for container in containers:
        encoded = ResponsesAPIRequestUtils._build_container_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L713
            container["custom_llm_provider"], container["model_id"], container["container_id"]
        )
        builds.append(
            {
                "encoded": encoded,
                "decoded": _decoded(
                    ResponsesAPIRequestUtils._decode_container_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L730
                        encoded
                    )
                ),
            }
        )
    decodes: list[Json] = []
    for encoded in ids:
        with Diagnostics() as diagnostics:
            container_decoded = ResponsesAPIRequestUtils._decode_container_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/utils.py#L730
                encoded
            )
            diagnostic = diagnostics.take()
        decodes.append(
            {
                "decoded": _decoded(container_decoded),
                "diagnostic": diagnostic,
            }
        )
    return {"containers": builds, "ids": decodes}


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
