# betterlite

betterlite is a LiteLLM-compatible AI gateway written in Rust. It is
rewriting LiteLLM 1.105.x, pinned at 1.105.1
([d665dc1a](https://github.com/BerriAI/litellm/tree/d665dc1a6315847fcad6cc2c1264ceb47c06abce)):
the proxy and the Python SDK, everything outside LiteLLM's `enterprise/`
folder.

It is MIT licensed; the repository's full license text is in
[LICENSE](LICENSE). Some behavior and data are reimplemented from LiteLLM
under its MIT license. The corresponding notice is in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

> [!CAUTION]
> LiteLLM's `enterprise/` folder is not MIT. LiteLLM's
> [LICENSE](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/LICENSE)
> puts it under BerriAI's Enterprise license, so betterlite contains none of
> that code and nobody working on betterlite reads it. LiteLLM's
> documentation lives in a separate repository,
> [BerriAI/litellm-docs](https://github.com/BerriAI/litellm-docs), under MIT,
> and it describes the enterprise features too. Some of them may be built
> later from those docs alone.
