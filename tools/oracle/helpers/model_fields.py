import json
from datetime import date
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ModelFieldError(ValueError):
    pass


_MODE_VALUES = {
    "audio_speech",
    "audio_transcription",
    "chat",
    "completion",
    "embedding",
    "evaluation",
    "guardrail",
    "image_edit",
    "image_generation",
    "moderation",
    "ocr",
    "realtime",
    "rerank",
    "responses",
    "search",
    "vector_store",
    "video_generation",
}
_MODALITY_VALUES = {"text", "image", "audio", "video", "code"}
_AUDIO_FORMAT_VALUES = {"mp3", "wav"}
_REASONING_EFFORT_VALUES = {"none", "minimal", "low", "medium", "high", "xhigh", "max"}
_VERTEX_AUDIO_API_VALUES = {"lyria_predict", "lyria_interactions"}
_WEB_SEARCH_BILLING_UNIT_VALUES = {"per_query", "per_prompt"}

_CAPABILITY_NAMES = [
    "supports_reasoning",
    "supports_legacy_thinking",
    "supports_adaptive_thinking",
    "supports_anthropic_thinking_payload",
    "supports_anthropic_compaction",
    "supports_none_reasoning_effort",
    "supports_minimal_reasoning_effort",
    "supports_low_reasoning_effort",
    "supports_max_reasoning_effort",
    "supports_xhigh_reasoning_effort",
    "supports_thinking_cache_preservation",
    "thinking_always_on",
    "supports_audio_input",
    "supports_audio_output",
    "supports_vision",
    "supports_image_input",
    "supports_video_input",
    "supports_pdf_input",
    "supports_embedding_image_input",
    "supports_multimodal",
    "supports_image_size",
    "supports_nova_canvas_image_edit",
    "gemini_audio_only_live",
    "gemini_native_audio",
    "uses_embed_content",
    "supports_function_calling",
    "supports_parallel_function_calling",
    "supports_forced_tool_use",
    "supports_tool_choice",
    "supports_tool_search",
    "supports_computer_use",
    "supports_web_search",
    "supports_url_context",
    "supports_response_schema",
    "supports_native_structured_output",
    "supports_sampling_params",
    "supports_assistant_prefill",
    "supports_system_messages",
    "supports_mid_conversation_system",
    "supports_prompt_caching",
    "supports_prompt_cache_breakpoint",
    "supports_native_streaming",
    "supports_speed",
    "supports_fast_mode",
    "bedrock_converse_supports_strict_tools",
    "supports_bedrock_runtime_chat_completions_response_format",
    "supports_bedrock_runtime_chat_completions_tools_with_reasoning",
    "use_openai_responses_path",
    "supports_output_config",
    "supports_parallel_tool_use_config",
]

_METADATA_NAMES = [
    "litellm_provider",
    "mode",
    "max_input_tokens",
    "max_output_tokens",
    "max_tokens",
    "rpm",
    "tpm",
    "prompt_cache_min_tokens",
    "output_vector_size",
    "supported_endpoints",
    "supported_regions",
    "supported_modalities",
    "supported_output_modalities",
    "supported_audio_formats",
    "audio_transcription_config",
    "default_reasoning_effort",
    "reasoning_effort_levels",
    "bedrock_output_config_effort_ceiling",
    "vertex_ai_audio_api",
    "web_search_billing_unit",
    "deprecation_date",
    "source",
    "comment",
    "metadata",
]

_NOTES_NAMES = ["calculation", "comment", "note", "notes", "successor", "supports_tool_choice"]
_INTEGER_FIELDS = [
    "max_input_tokens",
    "max_output_tokens",
    "max_tokens",
    "rpm",
    "tpm",
    "prompt_cache_min_tokens",
    "output_vector_size",
]
_STRING_LIST_FIELDS = ["supported_endpoints", "supported_regions"]
_SCALAR_STRING_FIELDS = ["audio_transcription_config", "source", "comment"]
_DATE_FIELDS = ["deprecation_date"]
_ENUM_FIELDS = {
    "mode": _MODE_VALUES,
    "default_reasoning_effort": _REASONING_EFFORT_VALUES,
    "bedrock_output_config_effort_ceiling": _REASONING_EFFORT_VALUES,
    "vertex_ai_audio_api": _VERTEX_AUDIO_API_VALUES,
    "web_search_billing_unit": _WEB_SEARCH_BILLING_UNIT_VALUES,
}
_ENUM_LIST_FIELDS = {
    "supported_modalities": _MODALITY_VALUES,
    "supported_output_modalities": _MODALITY_VALUES,
    "supported_audio_formats": _AUDIO_FORMAT_VALUES,
    "reasoning_effort_levels": _REASONING_EFFORT_VALUES,
}
_KNOWN_TOP_LEVEL = frozenset(_METADATA_NAMES) | frozenset(_CAPABILITY_NAMES)


def _string(path: str, value: Json) -> str:
    if not isinstance(value, str):
        raise ModelFieldError(f"model field {path}: must be a string")
    return value


def _enum(path: str, value: Json, allowed: set[str]) -> str:
    text = _string(path, value)
    if text not in allowed:
        raise ModelFieldError(f"model field {path}: unknown variant {text!r}")
    return text


def _integer(path: str, value: Json) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ModelFieldError(f"model field {path}: must be a non-negative integer")
    return value


def _boolean(path: str, value: Json) -> bool:
    if not isinstance(value, bool):
        raise ModelFieldError(f"model field {path}: must be a boolean")
    return value


def _list(path: str, value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ModelFieldError(f"model field {path}: must be a JSON array")
    return value


def _string_list(path: str, value: Json) -> list[Json]:
    items = _list(path, value)
    return [_string(f"{path}[{index}]", item) for index, item in enumerate(items)]


def _enum_list(path: str, value: Json, allowed: set[str]) -> list[Json]:
    items = _list(path, value)
    return [_enum(f"{path}[{index}]", item, allowed) for index, item in enumerate(items)]


def _date(path: str, value: Json) -> str:
    text = _string(path, value)
    try:
        date.fromisoformat(text)
    except ValueError as error:
        raise ModelFieldError(f"model field {path}: invalid calendar date") from error
    return text


def _notes(input_value: dict[str, Json]) -> dict[str, Json]:
    notes_value = input_value.get("metadata", {})
    if not isinstance(notes_value, dict):
        raise ModelFieldError("model field metadata: must be a JSON object")
    notes: dict[str, Json] = {}
    for name in ("calculation", "comment", "note", "notes", "successor"):
        if name in notes_value:
            notes[name] = _string(f"metadata.{name}", notes_value[name])
    if "supports_tool_choice" in notes_value:
        notes["supports_tool_choice"] = _boolean(
            "metadata.supports_tool_choice", notes_value["supports_tool_choice"]
        )
    return notes


def _unknown_fields(input_value: dict[str, Json]) -> list[str]:
    paths: list[str] = []
    for key, value in input_value.items():
        if key == "metadata":
            if isinstance(value, dict):
                paths.extend(f"metadata.{name}" for name in value if name not in _NOTES_NAMES)
            continue
        if key not in _KNOWN_TOP_LEVEL:
            paths.append(key)
    return paths


def _scalar_metadata_fields(input_value: dict[str, Json]) -> dict[str, Json]:
    fields: dict[str, Json] = {}
    for name in _INTEGER_FIELDS:
        if name in input_value:
            fields[name] = _integer(name, input_value[name])
    for name in _SCALAR_STRING_FIELDS:
        if name in input_value:
            fields[name] = _string(name, input_value[name])
    for name in _DATE_FIELDS:
        if name in input_value:
            fields[name] = _date(name, input_value[name])
    return fields


def _list_metadata_fields(input_value: dict[str, Json]) -> dict[str, Json]:
    fields: dict[str, Json] = {}
    for name in _STRING_LIST_FIELDS:
        if name in input_value:
            fields[name] = _string_list(name, input_value[name])
    for name, allowed in _ENUM_LIST_FIELDS.items():
        if name in input_value:
            fields[name] = _enum_list(name, input_value[name], allowed)
    return fields


def _enum_metadata_fields(input_value: dict[str, Json]) -> dict[str, Json]:
    fields: dict[str, Json] = {}
    for name, allowed in _ENUM_FIELDS.items():
        if name in input_value:
            fields[name] = _enum(name, input_value[name], allowed)
    return fields


def _capability_fields(input_value: dict[str, Json]) -> dict[str, Json]:
    return {
        name: _boolean(name, input_value[name])
        for name in _CAPABILITY_NAMES
        if name in input_value
    }


def _entry_fields(input_value: dict[str, Json]) -> dict[str, Json]:
    return {
        "litellm_provider": _string("litellm_provider", input_value.get("litellm_provider", "")),
        **_scalar_metadata_fields(input_value),
        **_list_metadata_fields(input_value),
        **_enum_metadata_fields(input_value),
        "metadata": _notes(input_value),
        **_capability_fields(input_value),
    }


def _entry(input_value: dict[str, Json]) -> dict[str, Json]:
    fields = _entry_fields(input_value)
    fields["unknown_fields"] = cast(Json, sorted(_unknown_fields(input_value)))
    return fields


def _fallback_generalization_rule(index: int, rule: Json) -> dict[str, Json]:
    if not isinstance(rule, dict):
        raise ModelFieldError(f"model field rules[{index}]: must be a JSON object")
    model_info = rule.get("model_info", {})
    if not isinstance(model_info, dict):
        raise ModelFieldError(f"model field rules[{index}].model_info: must be a JSON object")
    fields: dict[str, Json] = {
        "name": _string(f"rules[{index}].name", rule.get("name", "")),
        "pattern": _string(f"rules[{index}].pattern", rule.get("pattern", "")),
    }
    if rule.get("description") is not None:
        fields["description"] = rule["description"]
    if rule.get("extends") is not None:
        fields["extends"] = rule["extends"]
    if rule.get("fill_missing_for_providers"):
        fields["fill_missing_for_providers"] = rule["fill_missing_for_providers"]
    fields["model_info"] = _entry_fields(model_info)
    return fields


def _fallback_generalizations(input_value: dict[str, Json]) -> dict[str, Json]:
    rules_value = _list("rules", input_value.get("rules", []))
    return {
        "rules": [
            _fallback_generalization_rule(index, rule) for index, rule in enumerate(rules_value)
        ]
    }


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    source = input_value["source"]
    if not isinstance(source, str):
        raise ModelFieldError("model field source: must be a string")
    parsed = cast(dict[str, Json], json.loads(source))
    if input_value.get("kind") == "fallback_generalizations":
        return _fallback_generalizations(parsed)
    return _entry(parsed)
