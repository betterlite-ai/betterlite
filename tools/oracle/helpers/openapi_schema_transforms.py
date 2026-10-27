from typing import Any

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def rewrite(value: Json, renames: dict[str, str]) -> Json:
    if isinstance(value, list):
        return [rewrite(item, renames) for item in value]
    if not isinstance(value, dict):
        return value
    result: dict[str, Json] = {}
    for key, item in value.items():
        if key == "$defs":
            continue
        if key == "$ref" and isinstance(item, str) and item.startswith("#/$defs/"):
            name = item.removeprefix("#/$defs/")
            result[key] = f"#/components/schemas/{renames.get(name, name)}"
        else:
            result[key] = rewrite(item, renames)
    return result


def same_shape(existing: Json, incoming: Json) -> bool:
    if existing == incoming:
        return True
    if not isinstance(existing, dict) or not isinstance(incoming, dict):
        return False
    old = existing.get("properties")
    new = incoming.get("properties")
    if not isinstance(old, dict) or not isinstance(new, dict):
        return False
    return set(old) == set(new) and set(existing.get("required", [])) == set(incoming.get("required", []))


def run(input_value: dict[str, Any]) -> Json:
    function = input_value["function"]
    if function == "inline_request_body":
        schema = input_value.get("schema")
        media: dict[str, Json] = {"example": input_value.get("example")}
        if schema is None:
            return {"required": True, "content": {"application/json": media}}
        defs = schema.get("$defs", {}) if isinstance(schema, dict) else {}
        return {"required": True, "content": {"application/json": {"schema": inline(schema, defs), **media}}}
    document = input_value.get("openapi", {})
    if function == "add_schema_to_components":
        schema = input_value["schema"]
        name = input_value["schema_name"]
        components = document.setdefault("components", {}).setdefault("schemas", {})
        defs = schema.get("$defs", {})
        renames: dict[str, str] = {}
        while True:
            changed = False
            for key, value in defs.items():
                if key in components and not same_shape(components[key], rewrite(value, renames)):
                    renamed = f"{name}_{key}"
                    if renames.get(key) != renamed:
                        renames[key] = renamed
                        changed = True
            if not changed:
                break
        for key, value in defs.items():
            components[renames.get(key, key)] = rewrite(value, renames)
        components[name] = rewrite(schema, renames)
        return document
    if function == "add_request_body_to_paths":
        reference = input_value["schema_ref"].rsplit("/", 1)[-1]
        schema = document["components"]["schemas"][reference]
        expanded = {"type": "object", "required": schema.get("required", []), "properties": schema.get("properties", {})}
        for path in input_value["paths"]:
            post = document.get("paths", {}).get(path, {}).get("post")
            if post is None:
                continue
            post["requestBody"] = {"required": True, "content": {"application/json": {"schema": expanded}}}
            if "parameters" in post:
                post["parameters"] = [item for item in post["parameters"] if item.get("in") == "path"]
        return document
    raise ValueError(f"unknown function: {function}")


def inline(value: Json, defs: dict[str, Json], depth: int = 0) -> Json:
    if depth >= 32 or not isinstance(value, (dict, list)):
        return value
    if isinstance(value, list):
        return [inline(item, defs, depth + 1) for item in value]
    ref = value.get("$ref")
    if isinstance(ref, str) and ref.startswith("#/$defs/"):
        target = inline(defs[ref.removeprefix("#/$defs/")], defs, depth + 1)
        siblings = {key: inline(item, defs, depth + 1) for key, item in value.items() if key != "$ref"}
        return {**target, **siblings} if isinstance(target, dict) else siblings
    if "propertyName" in value and "mapping" in value:
        return {"propertyName": value["propertyName"]}
    return {key: inline(item, defs, depth + 1) for key, item in value.items() if key != "$defs"}
