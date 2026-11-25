from betterlite.types.utils.response import ModelResponseStream


def preserve_upstream_non_openai_attributes(  # ast-grep-ignore: no-long-names
    model_response: ModelResponseStream, original_chunk: ModelResponseStream
) -> None:
    expected = set(type(model_response).model_fields.keys()).union({"usage"})
    for key, value in original_chunk.model_dump().items():
        if key not in expected:
            model_response[key] = value
