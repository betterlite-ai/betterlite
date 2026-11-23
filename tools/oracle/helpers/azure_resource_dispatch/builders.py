from litellm.llms.azure.common_utils import BaseAzureLLM

import azure_batches.fake
import azure_files
import azure_fine_tuning

# a leaf saved whichever builder ran before it; restore the untouched one before its run
UNPATCHED_CLIENT_BUILDER = azure_batches.fake.REAL_AZURE_CLIENT  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489


def use_files_builder() -> None:
    azure_files.REAL_AZURE_CLIENT = UNPATCHED_CLIENT_BUILDER  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
    BaseAzureLLM.get_azure_openai_client = azure_files.build_client  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/files/handler.py#L53-L87


def use_batches_builder() -> None:
    azure_batches.fake.REAL_AZURE_CLIENT = UNPATCHED_CLIENT_BUILDER  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
    BaseAzureLLM.get_azure_openai_client = azure_batches.fake.build_client  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/batches/handler.py#L54-L61


def use_fine_tuning_builder() -> None:
    azure_fine_tuning.REAL_AZURE_CLIENT = UNPATCHED_CLIENT_BUILDER  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
    BaseAzureLLM.get_azure_openai_client = azure_fine_tuning.build_client  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/fine_tuning/handler.py#L197-L204
