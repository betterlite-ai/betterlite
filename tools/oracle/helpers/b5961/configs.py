import datetime as dt
from typing import Final

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.sagemaker.embedding.cohere_transformation import SagemakerCohereEmbeddingConfig
from litellm.llms.sagemaker.embedding.transformation import SagemakerEmbeddingConfig

HF: Final = SagemakerEmbeddingConfig()
COHERE: Final = SagemakerCohereEmbeddingConfig()
MODEL: Final = "sagemaker-endpoint"
LOG_START: Final = dt.datetime.fromtimestamp(1_700_000_000, dt.UTC)
CONFIG_NAMES: Final[dict[str, str]] = {
    "VoyageEmbeddingConfig": "voyage",
    "SagemakerCohereEmbeddingConfig": "cohere",
    "SagemakerEmbeddingConfig": "hugging_face",
}
LOG: Final = Logging(
    model=MODEL,
    messages=None,
    stream=False,
    call_type="embeddings",
    start_time=LOG_START,
    litellm_call_id="call-1",
    function_id="fn-1",
)
