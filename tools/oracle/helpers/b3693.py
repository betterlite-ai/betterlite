import re
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import MagicMock, mock_open, patch

from litellm.proxy.guardrails.guardrail_hooks.litellm_content_filter.content_filter import (
    ContentFilterGuardrail,
)
from litellm.types.guardrails import ContentFilterAction, ContentFilterPattern

if TYPE_CHECKING:
    from litellm.proxy.guardrails.guardrail_hooks.litellm_content_filter.content_filter import (
        CompiledPatternEntry,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Probe(ContentFilterGuardrail):
    def evaluate(self, data: dict[str, Json]) -> Json:
        result: Json
        function = data["function"]
        if function == "add":
            result = self.add_patterns(data)
        elif function == "spans":
            keyword = cast(str | None, data["keyword"])
            entry: CompiledPatternEntry = {
                "regex": re.compile(cast(str, data["pattern"]), re.IGNORECASE),
                "pattern_name": "custom_regex",
                "action": ContentFilterAction.MASK,
                "keyword_regex": re.compile(keyword, re.IGNORECASE)
                if keyword is not None
                else None,
                "allow_word_numbers": cast(bool, data["allow_word_numbers"]),
            }
            result = cast(Json, self._find_pattern_spans(cast(str, data["text"]), entry))
        elif function == "merge":
            merged = self._merge_spans(cast(list[tuple[int, int]], data["spans"]))
            masked = self._mask_spans(cast(str, data["text"]), merged, cast(str, data["tag"]))
            result = {"spans": cast(Json, merged), "masked": masked}
        elif function == "words":
            result = self.load_words(cast(str | None, data["content"]))
        elif function == "keyword":
            source = self._keyword_to_regex_pattern(cast(str, data["keyword"]))
            texts = cast(list[str], data["texts"])
            result = {
                "pattern": source,
                "matches": [re.search(source, text, re.IGNORECASE) is not None for text in texts],
            }
        elif function == "convert":
            result = self._convert_word_number_sequence(cast(str, data["sequence"]))
        elif function == "tag":
            self.pattern_redaction_format = cast(str, data["format"])
            result = self._mask_content("unused", cast(str, data["name"]))
        else:
            raise ValueError("unknown function")
        return result

    def load_words(self, content: str | None) -> Json:
        opener = cast(MagicMock, mock_open(read_data=content or ""))
        if content is None:
            opener.side_effect = FileNotFoundError("missing fixture")
        with patch("builtins.open", opener):
            self._load_blocked_words_file("blocked.yaml")
        return [
            {"keyword": key, "action": value[0].value, "description": value[1]}
            for key, value in sorted(self.blocked_words.items())
        ]

    def add_patterns(self, data: dict[str, Json]) -> Json:
        configs = cast(list[dict[str, Json]], data["patterns"])
        for config in configs:
            self._add_pattern(
                ContentFilterPattern.model_construct(
                    pattern_type=config["pattern_type"],
                    action=config["action"],
                    pattern_name=config.get("pattern_name"),
                    pattern=config.get("pattern"),
                    name=config.get("name"),
                )
            )
        text = cast(str, data["text"])
        entries: list[Json] = [
            {
                "name": entry["pattern_name"],
                "action": entry["action"],
                "allow_word_numbers": entry["allow_word_numbers"],
                "has_keyword": entry["keyword_regex"] is not None,
                "spans": cast(Json, self._find_pattern_spans(text, entry)),
            }
            for entry in self.compiled_patterns
        ]
        return {"patterns": entries, "first": cast(Json, self._check_patterns(text))}


def run(input_value: dict[str, Json]) -> Json:
    guardrail = Probe.__new__(Probe)
    guardrail.compiled_patterns = []
    guardrail.blocked_words = {}
    return guardrail.evaluate(input_value)
