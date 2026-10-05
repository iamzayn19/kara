#!/bin/sh
# Print registry fields (revision, size, sha256, license) for a GGUF file on
# Hugging Face, for models/registry.toml.
#   scripts/model-info.sh ggml-org/Qwen3-4B-GGUF Qwen3-4B-Q4_K_M.gguf
set -eu
exec python3 "$(dirname "$0")/model_info.py" "${1:?usage: model-info.sh <repo> <file>}" "${2:?usage: model-info.sh <repo> <file>}"
