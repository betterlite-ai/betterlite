from typing import ClassVar

from pydantic import ConfigDict, JsonValue

from betterlite.types.utils.base import ResponseObject


class MirroredPricingParams(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    input_cost_per_token: float | None = None
    output_cost_per_token: float | None = None
    input_cost_per_character: float | None = None
    output_cost_per_character: float | None = None
    cache_read_input_token_cost: float | None = None
    cache_creation_input_token_cost: float | None = None  # ast-grep-ignore: no-long-names
    tiered_pricing: list[dict[str, JsonValue]] | None = None


class CustomPricingLiteLLMParams(MirroredPricingParams):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    cost_per_second: float | None = None
    input_cost_per_second: float | None = None
    output_cost_per_second: float | None = None
    output_cost_per_second_1080p: float | None = None
    output_cost_per_second_480p: float | None = None
    output_cost_per_second_720p: float | None = None
    output_cost_per_second_768p: float | None = None
    output_cost_per_second_2k: float | None = None
    output_cost_per_second_4k: float | None = None
    output_cost_per_image_512: float | None = None
    output_cost_per_image_1024: float | None = None
    output_cost_per_image_1536: float | None = None
    input_cost_per_pixel: float | None = None
    output_cost_per_pixel: float | None = None
    input_cost_per_token_flex: float | None = None
    input_cost_per_token_priority: float | None = None
    input_cost_per_token_balanced: float | None = None
    input_cost_per_token_ultrafast: float | None = None
    cache_creation_input_token_cost_above_1hr: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_200k_tokens: float | None = None
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_272k_tokens: float | None = None
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_272k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_272k_tokens_flex: float | None = None
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_272k_tokens_ultrafast: float | None = None
    cache_creation_input_token_cost_flex: float | None = None  # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_priority: float | None = None  # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_ultrafast: float | None = None  # ast-grep-ignore: no-long-names
    cache_creation_input_audio_token_cost: float | None = None  # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_flex: float | None = None  # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_priority: float | None = None  # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_balanced: float | None = None  # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_ultrafast: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_200k_tokens: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_200k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_272k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_272k_tokens_flex: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_272k_tokens_ultrafast: float | None = None
    cache_read_input_token_cost_batches: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_200k_tokens_batches: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_272k_tokens_batches: float | None = None
    cache_creation_input_token_cost_batches: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_200k_tokens_batches: float | None = None
    # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_272k_tokens_batches: float | None = None
    cache_read_input_audio_token_cost: float | None = None  # ast-grep-ignore: no-long-names
    cache_read_input_image_token_cost: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    input_cost_per_character_above_128k_tokens: float | None = None
    input_cost_per_audio_token: float | None = None
    input_cost_per_token_cache_hit: float | None = None
    input_cost_per_token_above_128k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    input_cost_per_token_above_200k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    input_cost_per_token_above_200k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_token_above_272k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_token_above_272k_tokens_flex: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_token_above_272k_tokens_ultrafast: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_token_above_200k_tokens_batches: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_token_above_272k_tokens_batches: float | None = None
    input_cost_per_query: float | None = None
    input_cost_per_image: float | None = None
    input_cost_per_image_above_128k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    input_cost_per_audio_per_second: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    input_cost_per_audio_per_second_above_128k_tokens: float | None = None
    input_cost_per_video_per_second: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    input_cost_per_video_per_second_above_128k_tokens: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_video_per_second_above_15s_interval: float | None = None
    # ast-grep-ignore: no-long-names
    input_cost_per_video_per_second_above_8s_interval: float | None = None
    input_cost_per_audio_token_batches: float | None = None  # ast-grep-ignore: no-long-names
    input_cost_per_image_token_batches: float | None = None  # ast-grep-ignore: no-long-names
    input_cost_per_token_batches: float | None = None
    input_cost_per_video_token_batches: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_token_batches: float | None = None
    output_cost_per_token_flex: float | None = None
    output_cost_per_token_priority: float | None = None
    output_cost_per_token_balanced: float | None = None
    output_cost_per_token_ultrafast: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_audio_token: float | None = None
    output_cost_per_token_above_128k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_token_above_200k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    # ast-grep-ignore: no-long-names
    output_cost_per_token_above_200k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    output_cost_per_token_above_272k_tokens_priority: float | None = None
    # ast-grep-ignore: no-long-names
    output_cost_per_token_above_272k_tokens_flex: float | None = None
    # ast-grep-ignore: no-long-names
    output_cost_per_token_above_272k_tokens_ultrafast: float | None = None
    # ast-grep-ignore: no-long-names
    output_cost_per_token_above_200k_tokens_batches: float | None = None
    # ast-grep-ignore: no-long-names
    output_cost_per_token_above_272k_tokens_batches: float | None = None
    # ast-grep-ignore: no-long-names
    output_cost_per_character_above_128k_tokens: float | None = None
    output_cost_per_image: float | None = None
    output_cost_per_image_token: float | None = None
    output_cost_per_video_token: float | None = None
    output_cost_per_reasoning_token: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_reasoning_token_flex: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_reasoning_token_priority: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_video_per_second: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_audio_per_second: float | None = None  # ast-grep-ignore: no-long-names
    search_context_cost_per_query: dict[str, JsonValue] | None = None
    google_maps_grounding_cost_per_query: float | None = None  # ast-grep-ignore: no-long-names
    citation_cost_per_token: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_272k_tokens: float | None = None
    # ast-grep-ignore: no-long-names
    cache_read_input_token_cost_above_512k_tokens: float | None = None
    input_cost_per_image_token: float | None = None
    input_cost_per_video_token: float | None = None
    input_cost_per_token_above_272k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    input_cost_per_token_above_512k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_token_above_272k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    output_cost_per_token_above_512k_tokens: float | None = None  # ast-grep-ignore: no-long-names
    output_vector_size: int | None = None
    ocr_cost_per_page: float | None = None
    ocr_cost_per_page_batches: float | None = None
    ocr_cost_per_credit: float | None = None
    annotation_cost_per_page: float | None = None
    annotation_cost_per_page_batches: float | None = None  # ast-grep-ignore: no-long-names
    regional_processing_uplift_multiplier_eu: float | None = None  # ast-grep-ignore: no-long-names
    regional_processing_uplift_multiplier_us: float | None = None  # ast-grep-ignore: no-long-names
    regional_endpoint_uplift_multiplier: float | None = None  # ast-grep-ignore: no-long-names

    @classmethod
    def strip_custom_pricing_fields(cls, model_info: dict[str, JsonValue]) -> dict[str, JsonValue]:
        return {key: value for key, value in model_info.items() if key not in cls.model_fields}
