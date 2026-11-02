import tempfile
from pathlib import Path
from typing import TypeAlias, TypedDict, cast
from unittest.mock import patch

from litellm._logging import verbose_proxy_logger
from litellm.proxy.guardrails.guardrail_hooks.litellm_content_filter import patterns

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CategoryFile(TypedDict):
    filename: str
    content: str


def _categories(roots: list[list[CategoryFile]]) -> Json:
    warnings: list[str] = []

    def warning(_message: str, filename: str, error: object) -> None:
        if str(error):
            warnings.append(filename)

    with tempfile.TemporaryDirectory() as temporary:
        paths: list[str] = []
        for index, files in enumerate(roots):
            root = Path(temporary) / str(index)
            directory = root / "categories"
            directory.mkdir(parents=True)
            paths.append(str(root))
            for entry in files:
                filename = entry["filename"]
                if Path(filename).name != filename:
                    raise ValueError("fixture filename must be a basename")
                (directory / filename).write_text(entry["content"])
        with patch.object(verbose_proxy_logger, "warning", side_effect=warning):
            categories = patterns.get_available_content_categories(tuple(paths))
        return {"categories": cast(Json, categories), "warnings": cast(Json, warnings)}


def run(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    if function == "catalog":
        return {
            "names": cast(Json, patterns.get_all_pattern_names()),
            "metadata": cast(Json, patterns.get_pattern_metadata()),
            "patterns": cast(Json, patterns.PREBUILT_PATTERNS),
            "categories": cast(Json, patterns.PATTERN_CATEGORIES),
            "display_names": cast(Json, patterns.PATTERN_DISPLAY_NAMES),
            "descriptions": cast(Json, patterns.PATTERN_DESCRIPTIONS),
            "extra_config": cast(Json, patterns.PATTERN_EXTRA_CONFIG),
        }
    if function == "compile":
        name = cast(str, input_value["name"])
        texts = cast(list[str], input_value["texts"])
        compiled = patterns.get_compiled_pattern(name)
        return [compiled.search(text) is not None for text in texts]
    if function == "categories":
        return _categories(cast(list[list[CategoryFile]], input_value["roots"]))
    raise ValueError(f"unknown pattern discovery function: {function}")
