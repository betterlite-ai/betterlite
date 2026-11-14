from copy import deepcopy
from typing import Protocol

from pydantic import JsonValue


class ResponseFormatModel(Protocol):
    @classmethod
    def model_json_schema(
        cls, *, ref_template: str = "#/$defs/{model}",
    ) -> dict[str, JsonValue]: ...


def _as_dict(value: JsonValue) -> dict[str, JsonValue] | None:
    match value:
        case dict() as mapping:
            return mapping
        case bool() | int() | float() | str() | list() | None:
            return None


def _as_list(value: JsonValue) -> list[JsonValue] | None:
    match value:
        case list() as items:
            return items
        case bool() | int() | float() | str() | dict() | None:
            return None


def _is_container(value: JsonValue) -> bool:
    match value:
        case dict() | list():
            return True
        case bool() | int() | float() | str() | None:
            return False


def _container_children(value: JsonValue) -> list[JsonValue]:
    match value:
        case dict():
            return [child for child in value.values() if _is_container(child)]
        case list():
            return [child for child in value if _is_container(child)]
        case bool() | int() | float() | str() | None:
            return []


def _rewrite_ref(node: dict[str, JsonValue], ref_template: str) -> None:
    ref = node.get("$ref")
    match ref:
        case str():
            node["$ref"] = ref_template.format(model=ref.rsplit("/", 1)[-1])
        case bool() | int() | float() | list() | dict() | None:
            pass


def _rewrite_refs(schema: dict[str, JsonValue], ref_template: str) -> None:
    stack: list[JsonValue] = [schema]
    visited: set[int] = set()
    while stack:
        node = stack.pop()
        if id(node) in visited:
            continue
        visited.add(id(node))
        match node:
            case dict():
                _rewrite_ref(node, ref_template)
            case bool() | int() | float() | str() | list() | None:
                pass
        stack.extend(_container_children(node))


def _dict_to_response_format_helper(  # ast-grep-ignore: no-long-names
    response_format: dict[str, JsonValue], ref_template: str | None = None,
) -> dict[str, JsonValue]:
    if ref_template is None or response_format.get("type") != "json_schema":
        return response_format
    modified_format = deepcopy(response_format)
    json_schema = _as_dict(modified_format.get("json_schema"))
    if json_schema is None:
        raise TypeError("json_schema is not a dictionary")
    schema = _as_dict(json_schema.get("schema"))
    if schema is None:
        raise TypeError("json_schema.schema is not a dictionary")
    _rewrite_refs(schema, ref_template)
    return modified_format


def resolve_ref(*, root: dict[str, JsonValue], ref: str) -> JsonValue:
    if not ref.startswith("#/"):
        raise ValueError(f"Unexpected $ref format {ref!r}; Does not start with #/")
    resolved: JsonValue = root
    for key in ref[2:].split("/"):
        entry = _as_dict(resolved)
        if entry is None:
            raise ValueError(f"encountered non-dictionary entry while resolving {ref} - {resolved}")
        resolved = entry[key]
    return resolved


def _strictify_defs(
    schema: dict[str, JsonValue], *, path: tuple[str, ...], root: dict[str, JsonValue],
) -> None:
    defs = _as_dict(schema.get("$defs"))
    if defs is not None:
        for name, definition in defs.items():
            _ensure_strict_json_schema(definition, path=(*path, "$defs", name), root=root)
    definitions = _as_dict(schema.get("definitions"))
    if definitions is not None:
        for name, definition in definitions.items():
            _ensure_strict_json_schema(definition, path=(*path, "definitions", name), root=root)


def _strictify_properties(
    schema: dict[str, JsonValue], *, path: tuple[str, ...], root: dict[str, JsonValue],
) -> None:
    if schema.get("type") == "object" and "additionalProperties" not in schema:
        schema["additionalProperties"] = False
    properties = _as_dict(schema.get("properties"))
    if properties is not None:
        schema["required"] = list(properties.keys())
        for name, property_schema in properties.items():
            properties[name] = _ensure_strict_json_schema(
                property_schema, path=(*path, "properties", name), root=root,
            )


def _strictify_all_of(
    schema: dict[str, JsonValue], all_of: list[JsonValue], *,
    path: tuple[str, ...], root: dict[str, JsonValue],
) -> None:
    if len(all_of) == 1:
        merged = _ensure_strict_json_schema(all_of[0], path=(*path, "allOf", "0"), root=root)
        schema.update(merged)
        schema.pop("allOf")
        return
    schema["allOf"] = [
        _ensure_strict_json_schema(entry, path=(*path, "allOf", str(index)), root=root)
        for index, entry in enumerate(all_of)
    ]


def _strictify_composition(
    schema: dict[str, JsonValue], *, path: tuple[str, ...], root: dict[str, JsonValue],
) -> None:
    items = _as_dict(schema.get("items"))
    if items is not None:
        schema["items"] = _ensure_strict_json_schema(items, path=(*path, "items"), root=root)
    any_of = _as_list(schema.get("anyOf"))
    if any_of is not None:
        schema["anyOf"] = [
            _ensure_strict_json_schema(variant, path=(*path, "anyOf", str(index)), root=root)
            for index, variant in enumerate(any_of)
        ]
    all_of = _as_list(schema.get("allOf"))
    if all_of is not None:
        _strictify_all_of(schema, all_of, path=path, root=root)


def _inline_ref(
    schema: dict[str, JsonValue], ref: str, *,
    path: tuple[str, ...], root: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    resolved = _as_dict(resolve_ref(root=root, ref=ref))
    if resolved is None:
        raise ValueError(f"Expected `$ref: {ref}` to resolved to a dictionary")
    schema.update({**resolved, **schema})
    schema.pop("$ref")
    return _ensure_strict_json_schema(schema, path=path, root=root)


def _strictify_ref(
    schema: dict[str, JsonValue], *, path: tuple[str, ...], root: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    if "default" in schema and schema["default"] is None:
        schema.pop("default")
    ref = schema.get("$ref")
    match ref:
        case str() if len(schema) > 1:
            return _inline_ref(schema, ref, path=path, root=root)
        case None | str():
            return schema
        case bool() | int() | float() | list() | dict():
            raise TypeError(f"Received non-string $ref - {ref}")


def _ensure_strict_json_schema(
    json_schema: JsonValue, *, path: tuple[str, ...], root: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    match json_schema:
        case dict():
            _strictify_defs(json_schema, path=path, root=root)
            _strictify_properties(json_schema, path=path, root=root)
            _strictify_composition(json_schema, path=path, root=root)
            return _strictify_ref(json_schema, path=path, root=root)
        case bool() | int() | float() | str() | list() | None:
            raise TypeError(f"Expected {json_schema} to be a dictionary; path={path}")


def _class_schema(
    response_class: type[ResponseFormatModel], ref_template: str | None,
) -> dict[str, JsonValue]:
    try:
        schema_method = response_class.model_json_schema
    except AttributeError as error:
        raise TypeError(f"Unsupported response_format type - {response_class}") from error
    if ref_template is not None:
        return schema_method(ref_template=ref_template)
    schema = schema_method()
    return _ensure_strict_json_schema(schema, path=(), root=schema)


def type_to_response_format_param(
    response_format: type[ResponseFormatModel] | JsonValue,
    ref_template: str | None = None,
) -> dict[str, JsonValue] | None:
    if response_format is None:
        return None
    match response_format:
        case dict():
            return _dict_to_response_format_helper(response_format, ref_template)
        case int() | float() | str() | list():
            raise TypeError(f"Unsupported response_format type - {response_format}")
        case type():  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_utils.py#L179-L212
            return {
                "type": "json_schema",
                "json_schema": {
                    "schema": _class_schema(response_format, ref_template),
                    "name": response_format.__name__,
                    "strict": True,
                },
            }
    raise TypeError(f"Unsupported response_format type - {response_format}")
