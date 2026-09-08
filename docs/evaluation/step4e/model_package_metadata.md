# STEP4E optional Candle model-package verification

- Model: `BAAI/bge-small-en-v1.5`
- Model version: `BAAI/bge-small-en-v1.5` (upstream `main`, pinned by content hash)
- Resolution path: `/home/panda/.local/share/amatl/models/bge-small-en-v1.5`
- Environment variable used by the infrastructure test: `AMATL_TEST_MODEL_DIR`
- Production loader contract: explicit local `MODEL_PATH` and `MODEL_VERSION`; it never downloads.

Required files and SHA-256 values:

| File | SHA-256 |
| --- | --- |
| `model.safetensors` | `3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad` |
| `tokenizer.json` | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` |
| `config.json` | `094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750` |

Source: the authoritative `https://huggingface.co/BAAI/bge-small-en-v1.5/resolve/main/`
distribution. All three downloaded files matched the pinned hashes exactly.

The real Candle load validation used only `local embedding package verification`.
It passed with a 384-dimensional, finite output; `MODEL_LOAD_MS=3125` on this host.
The binary package remains outside Git.
