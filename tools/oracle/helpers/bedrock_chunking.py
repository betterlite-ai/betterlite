# ruff: noqa: C901, PLR0911, PLR0912, PLR0915, PLR2004

from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def text(item: dict[str, Json]) -> str:
    value = item.get("text")
    return cast(
        str,
        value.get("text", "")  # pyright: ignore[reportUnknownMemberType]
        if isinstance(value, dict) and isinstance(value.get("text", ""), str)
        else "",
    )


def nearest(value: str) -> int:
    midpoint = len(value) // 2
    candidates = [
        index + 1 for index, character in enumerate(value[:midpoint]) if character.isspace()
    ]
    candidates += [
        index + 1
        for index, character in enumerate(value[midpoint:], midpoint)
        if character.isspace()
    ]
    safe = [index for index in candidates if 0 < index < len(value)]
    return min(safe, key=lambda index: abs(index - midpoint)) if safe else midpoint


def run(input_value: dict[str, Json]) -> Json:
    step = input_value.get("step")
    if not isinstance(step, dict):
        raise ValueError("step must be an object")
    operation = step.get("operation")
    if operation == "bin_pack":
        content = step.get("content")
        budget = step.get("budget")
        if not isinstance(content, list) or not isinstance(budget, int):
            raise ValueError("bin_pack requires content and budget")
        batches: list[list[Json]] = []
        current: list[Json] = []
        used = 0
        for item in content:
            if not isinstance(item, dict):
                raise ValueError("content items must be objects")
            length = len(text(item))
            if current and used + length > budget:
                batches.append(current)
                current, used = [], 0
            current.append(item)
            used += length
        if current:
            batches.append(current)
        return cast(Json, batches if batches else [list[Json]()])  # noqa: FURB110
    if operation == "nearest":
        value = step.get("text")
        if not isinstance(value, str):
            raise ValueError("nearest requires text")
        return nearest(value)
    if operation == "split":
        content = step.get("content")
        if not isinstance(content, list):
            raise ValueError("split requires content")
        if len(content) > 1:
            midpoint = len(content) // 2
            return [content[:midpoint], content[midpoint:]]
        if not content or not isinstance(content[0], dict):
            return None
        item = content[0]
        source = text(item)
        if len(source) < 2:
            return None
        index = nearest(source)
        left, right = source[:index], source[index:]
        original = item.get("text")
        qualifiers = original.get("qualifiers") if isinstance(original, dict) else None
        return [
            [{"text": {"text": left, **({"qualifiers": qualifiers} if qualifiers else {})}}],
            [{"text": {"text": right, **({"qualifiers": qualifiers} if qualifiers else {})}}],
        ]
    if operation == "merge":
        chunks = step.get("chunks")
        if not isinstance(chunks, list):
            raise ValueError("merge requires chunks")
        merged: dict[str, Json] = {}
        outputs: list[Json] = []
        actions: list[str] = []
        masked = False
        for chunk in chunks:
            if not isinstance(chunk, dict) or not isinstance(chunk.get("response"), dict):
                raise ValueError("invalid chunk")
            response = cast(dict[str, Json], chunk["response"])
            if isinstance(response.get("action"), str):
                actions.append(cast(str, response["action"]))
            values = response.get("outputs", response.get("output", []))
            content = chunk.get("content", [])
            if not isinstance(content, list):
                content = []
            if isinstance(values, list) and values:
                masked = True
            if isinstance(values, list) and len(values) == len(content):
                outputs.extend(values)
            elif isinstance(values, list) and not values:
                outputs.extend({"text": text(item)} for item in content if isinstance(item, dict))
            if isinstance(response.get("usage"), dict):
                usage_values = cast(dict[str, Json], response["usage"])
                usage = merged.setdefault("usage", {})
                if isinstance(usage, dict):
                    for key, value in usage_values.items():
                        if isinstance(value, int):
                            previous = usage.get(key, 0)
                            usage[key] = int(cast(int, previous)) + cast(int, value)
        if actions:
            merged["action"] = (
                "GUARDRAIL_INTERVENED" if "GUARDRAIL_INTERVENED" in actions else actions[-1]
            )
        if masked:
            merged["outputs"] = outputs
            merged["output"] = outputs
        return merged
    raise ValueError(f"unsupported operation: {operation}")
