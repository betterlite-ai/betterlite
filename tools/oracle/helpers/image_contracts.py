import asyncio
from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.base_llm.image_generation.transformation import BaseImageGenerationConfig
from litellm.llms.base_llm.image_variations.transformation import BaseImageVariationConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import FileTypes
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
TEXT: TypeAdapter[str] = TypeAdapter(str)
CONFIG: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
BASES: TypeAdapter[list[str | None]] = TypeAdapter(list[str | None])
TEXTS: TypeAdapter[dict[str, str]] = TypeAdapter(dict[str, str])
Parts: TypeAlias = list[tuple[str, tuple[str | None, bytes, str | None]]]
FILES: TypeAdapter[Parts] = TypeAdapter(Parts)
MODEL = "dall-e-2"
EMPTY: dict[str, JsonValue] = {}
MESSAGES: list[AllMessageValues] = []
GEN = BaseImageGenerationConfig
VAR = BaseImageVariationConfig
EDIT = BaseImageEditConfig
ClassKind = Literal[
    "value", "function", "builtin_function", "class_method", "static_method",
    "property", "callable_object",
]


class ClassEntry(TypedDict):
    name: str
    kind: ClassKind
    value: JsonValue
    inherited: bool


class Scenario(TypedDict):
    model: str
    prompt: str | None
    options: dict[str, JsonValue]
    headers: dict[str, str]
    raise_status: int | None


SCENARIOS: TypeAdapter[list[Scenario]] = TypeAdapter(list[Scenario])
ENTRIES: TypeAdapter[list[ClassEntry]] = TypeAdapter(list[ClassEntry])


RAISED = "the fixture provider failed"


class FixtureProviderError(Exception): ...


def stub(*_args: JsonValue) -> JsonValue: ...


Attr: TypeAlias = JsonValue | Callable[..., JsonValue] | property  # type: ignore[explicit-any]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
GEN_NS: dict[str, Attr] = {"get_supported_openai_params": stub, "map_openai_params": stub}
VAR_NS: dict[str, Attr] = {
    "get_supported_openai_params": stub, "map_openai_params": stub, "get_error_class": stub,
    "transform_request_image_variation": stub, "transform_response_image_variation": stub,
    "async_transform_response_image_variation": stub,
}
GENERATION: type[BaseImageGenerationConfig] = type("GenerationFixture", (GEN,), GEN_NS)
VARIATION: type[BaseImageVariationConfig] = type("VariationFixture", (VAR,), VAR_NS)
ATTRIBUTES: dict[ClassKind, Attr] = {
    "function": stub, "builtin_function": len, "class_method": stub, "static_method": stub,
    "property": property(stub), "callable_object": int,
}


class EditState:
    def __init__(self, raise_status: int | None) -> None:
        self.calls = 0
        self.seen: list[dict[str, JsonValue]] = []
        self.raise_status = raise_status


class EditInstance(Protocol):
    state: EditState


def record_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
    self: EditInstance,
    model: str,
    prompt: str | None,
    image: FileTypes | None,
    params: dict[str, JsonValue],
    _litellm_params: JsonValue,
    headers: dict[str, str],
) -> JsonValue:
    state = self.state
    if image is not None:
        raise ValueError("the fixture records only a missing image")
    seen: dict[str, JsonValue] = {"model": model, "prompt": prompt, "image": None,
                                   "options": dict(params), "headers": dict(headers)}
    state.seen.append(seen)
    state.calls += 1
    if state.raise_status is not None:
        raise FixtureProviderError(RAISED)
    data = dict(params)
    data["touched"] = True
    request_headers = dict(headers)
    request_headers["touched"] = "yes"
    data["header_touched"] = "touched" in request_headers
    return [data, [["image", ["a.png", [97], "image/png"]]]]


def edit_fixture(state: EditState) -> type[BaseImageEditConfig]:
    namespace: dict[str, Attr | EditState] = {
        "state": state,
        "get_supported_openai_params": stub,
        "map_openai_params": stub,
        "transform_image_edit_request": record_request,
        "transform_image_edit_response": stub,
        "validate_environment": stub,
        "get_complete_url": stub,
    }
    return type("EditFixture", (EDIT,), namespace)


def generation_environment(headers: dict[str, str]) -> JsonValue:
    return JSON.validate_python(
        GEN.validate_environment(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
            GENERATION(), headers=headers, model=MODEL,
            messages=MESSAGES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
            optional_params=EMPTY, litellm_params=EMPTY, api_key=None, api_base=None,
        )
    )


def transport_defaults(raw: dict[str, JsonValue]) -> JsonValue:
    generation = GENERATION()
    edit = edit_fixture(EditState(None))()
    headers = TEXTS.validate_python(raw["headers"])
    bases = BASES.validate_python(raw["api_bases"])
    edit_urls: list[JsonValue] = []
    for base in BASES.validate_python(raw["edit_bases"]):
        try:
            url = EDIT.get_complete_url(edit, MODEL, base, EMPTY)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
        except ValueError:
            edit_urls.append(None)
        else:
            edit_urls.append(JSON.validate_python(url))
    values: dict[str, JsonValue] = {
        "generation_urls": [
            JSON.validate_python(GEN.get_complete_url(generation, b, None, MODEL, EMPTY, EMPTY))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
            for b in bases
        ],
        "variation_urls": [
            JSON.validate_python(VAR.get_complete_url(VARIATION(), b, None, MODEL, EMPTY, EMPTY))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_variations/transformation.py
            for b in bases
        ],
        "edit_urls": edit_urls,
        "generation_environment": generation_environment(headers),
        "generation_multipart": GEN.use_multipart_form_data(generation),
        "edit_multipart": EDIT.use_multipart_form_data(edit),
        "finalized": JSON.validate_python(
            EDIT.finalize_image_edit_request_data(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
                edit,
                CONFIG.validate_python(raw["finalize_data"]),
                TEXT.validate_python(raw["finalize_url"]),
            )
        ),
    }
    return JSON.validate_python(values)


def part_value(name: str, part: tuple[str | None, bytes, str | None]) -> JsonValue:
    value: dict[str, JsonValue] = {"name": name, "filename": part[0], "content": list(part[1]),
                                   "content_type": part[2]}
    return JSON.validate_python(value)


def copy_scenario(scenario: Scenario) -> JsonValue:
    state = EditState(scenario["raise_status"])
    try:
        f = edit_fixture(state)()
        data, files = asyncio.run(f.async_transform_image_edit_request(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
                model=scenario["model"], prompt=scenario["prompt"], image=None,
                image_edit_optional_request_params=scenario["options"],
                litellm_params=None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
                headers=scenario["headers"],
            )
        )
    except FixtureProviderError as error:
        result: dict[str, JsonValue] = {"raised": str(error)}
    else:
        payload = JSON.validate_python(data)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py
        raw_parts = FILES.validate_python(files)
        parts = [part_value(name, part) for name, part in raw_parts]
        result = {"data": payload, "files": JSON.validate_python(parts)}
    values: dict[str, JsonValue] = {
        "calls": state.calls,
        "seen": [JSON.validate_python(seen) for seen in state.seen],
        "options": scenario["options"],
        "headers": JSON.validate_python(scenario["headers"]),
        "result": result,
    }
    return JSON.validate_python(values)


def edit_class_configuration(raw: dict[str, JsonValue]) -> JsonValue:
    entries = ENTRIES.validate_python(raw["entries"])
    own: dict[str, Attr] = {}
    inherited: dict[str, Attr] = {}
    for entry in entries:
        target = inherited if entry["inherited"] else own
        target[entry["name"]] = (
            entry["value"] if entry["kind"] == "value" else ATTRIBUTES[entry["kind"]]
        )
    parent = type("InheritedConfig", (edit_fixture(EditState(None)),), inherited)
    config_type: type[BaseImageEditConfig] = type("ConfiguredConfig", (parent,), own)
    config_raw = config_type.get_config()  # type: ignore[no-untyped-call, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
    config = CONFIG.validate_python(config_raw)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
    known = {entry["name"]: entry for entry in entries}
    values: dict[str, JsonValue] = {
        name: {"kind": "value", "value": known[name]["value"]}
        if known[name]["kind"] == "value"
        else {"kind": known[name]["kind"]}
        for name in config
    }
    result: dict[str, JsonValue] = {"config": values}
    return JSON.validate_python(result)


def error_probe(raw: dict[str, JsonValue], kind: str) -> JsonValue:
    message = TEXT.validate_python(raw["message"])
    status = TypeAdapter(int).validate_python(raw["status_code"])
    headers = TEXTS.validate_python(raw["headers"])
    try:
        if kind == "edit":
            EDIT.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py
                edit_fixture(EditState(None))(),
                error_message=message, status_code=status, headers=headers,
            )
        else:
            GEN.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
                GENERATION(), error_message=message, status_code=status, headers=headers
            )
    except BaseLLMException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py
        header_map = raised.headers  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py
        headers_value = list(header_map.items()) if header_map is not None else []  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py
        facts: dict[str, JsonValue] = {"status_code": raised.status_code, "message": raised.message,
                                       "headers": JSON.validate_python(headers_value)}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py
        return JSON.validate_python(facts)
    raise AssertionError("get_error_class returned without raising")


def probe(kind: str) -> JsonValue:
    try:
        if kind == "generation_request":
            GEN.transform_image_generation_request(GENERATION(), MODEL, "a cat", EMPTY, EMPTY, {})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
        if kind == "generation_response":
            GEN.transform_image_generation_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
                GENERATION(), MODEL, httpx.Response(200),
                None, None, EMPTY, EMPTY, EMPTY, None, None, None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_generation/transformation.py
            )
        if kind == "variation_request":
            VAR.transform_request(VARIATION(), MODEL, MESSAGES, EMPTY, EMPTY, {})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_variations/transformation.py
        if kind == "variation_response":
            VAR.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_variations/transformation.py
                VARIATION(), MODEL, httpx.Response(200),
                None, None, EMPTY, MESSAGES, EMPTY, EMPTY, None, None, None,  # type: ignore[arg-type, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_variations/transformation.py
            )
    except NotImplementedError as error:
        return str(error)
    return "no error"


def run(raw: dict[str, JsonValue]) -> JsonValue:
    operation = TEXT.validate_python(raw["operation"])
    if operation == "transport_defaults":
        return transport_defaults(raw)
    if operation == "edit_async_transform_copy":
        scenarios = SCENARIOS.validate_python(raw["scenarios"])
        cases: dict[str, JsonValue] = {"cases": [copy_scenario(scenario) for scenario in scenarios]}
        return JSON.validate_python(cases)
    if operation == "edit_class_configuration":
        return edit_class_configuration(raw)
    if operation == "base_errors":
        values: dict[str, JsonValue] = {
            "generation": error_probe(raw, "generation"),
            "edit": error_probe(raw, "edit"),
            "generation_request": probe("generation_request"),
            "generation_response": probe("generation_response"),
            "variation_request": probe("variation_request"),
            "variation_response": probe("variation_response"),
        }
        return JSON.validate_python(values)
    raise ValueError(f"unknown image_contracts operation: {operation}")
