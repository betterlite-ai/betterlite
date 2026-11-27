# betterlite

betterlite is LiteLLM's SDK and proxy rewritten in Rust, under the MIT license.
The Python package keeps LiteLLM's names, so code written for LiteLLM runs unchanged:

```python
import betterlite as litellm
```

## Install

```bash
pip install betterlite
```

Wheels ship for Linux (manylinux_2_28, x86_64 and aarch64), macOS (x86_64 and arm64) and Windows (x64).
They are cp310-abi3, so one wheel covers Python 3.10 and newer.

## What you get

The completion, embedding, router and proxy client calls you already use, with request handling,
the model cost map and the provider adapters compiled into a native extension.

## Links

- Source and issues: https://github.com/betterlite-ai/betterlite
- License: MIT, see https://github.com/betterlite-ai/betterlite/blob/main/LICENSE
