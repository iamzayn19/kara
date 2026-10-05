# Model support

Kara works with any model served through an OpenAI-compatible chat
completions endpoint that supports tool calling. It manages llama.cpp itself;
Ollama, LM Studio, vLLM and other local servers work through configuration.
The agent does not depend on any model family.

## Registry

Downloadable models are data in [`models/registry.toml`](models/registry.toml).
Each entry pins a Hugging Face repository, a commit revision, a file, its size
and SHA-256, the license, and the context and memory figures used for
planning. Add or override entries in `~/.kara/models.toml`.

| Tier | Model | Quantization | Download | Notes |
|---|---|---|---|---|
| high | Qwen3.6-35B-A3B | Q4_K_M | 20.4 GB | Mixture of experts, about 3B active parameters; experts can run from system RAM on smaller GPUs. Project default for capable hardware. |
| mid | Qwen3.6-27B | Q4_K_M | 19.1 GB | Dense; needs the whole model in fast memory. |
| mid-low | Qwen3-14B | Q4_K_M | 9.0 GB | |
| low | Qwen3-4B | Q4_K_M | 2.5 GB | Runs on 8-16 GB machines; thinking capped at 1024 tokens per step. |
| minimal | Qwen3-1.7B | Q4_K_M | 1.3 GB | Not auto-selected; for smoke tests. |
| mid | Qwen3.8-27B | Q4_K_M | 19.0 GB | Newer release; manual selection only until evaluated. |

All listed models are Apache-2.0. Kara never bundles model weights. Before a
download it shows the name, download size, estimated memory, license, source
and revision, and waits for your confirmation. Downloads resume after
interruption and are verified against the pinned SHA-256.

## Automatic selection (`/model auto`, `kara doctor`)

1. Detect hardware: OS, CPU, RAM, GPU and VRAM, Apple unified memory, Metal,
   CUDA, Vulkan, ROCm, and free disk.
2. Compute a fast-memory budget: about 70% of RAM on Apple Silicon (the GPU's
   default working-set limit), about 92% of VRAM on discrete GPUs, otherwise a
   share of RAM for CPU inference.
3. For each eligible model (permitted license, tool calling,
   `auto_select = true`), estimate weights + KV cache + overhead at its
   default context, halving the context down to its minimum before giving up.
   MoE models may instead run with experts in system RAM on a discrete GPU.
4. Pick the highest *measured* Kara eval score when every fitting model has
   one; otherwise rank all of them by the provisional `quality_rank`, so one
   measured model never outranks unmeasured ones. A model that fits only with
   offloading is ranked below one that fits fully.

Larger is not assumed to be better. Once `kara eval` results exist, record
`eval_score` in the registry so selection follows measured agent performance
(see [docs/MODEL_EVALUATION.md](docs/MODEL_EVALUATION.md)).

Memory figures (`kv_bytes_per_token`) are estimates. If a model fails to load,
lower `model.context_length` or pick a smaller model.

## Other runtimes

```toml
# ~/.kara/config.toml
[model]
mode = "manual"
provider = "ollama"          # ollama | lmstudio | vllm | openai_compat
api_model = "qwen3:8b"       # name as the server knows it
# endpoint = "http://127.0.0.1:11434/v1"   # defaults per provider
```

Default endpoints: Ollama `http://127.0.0.1:11434/v1`, LM Studio
`http://127.0.0.1:1234/v1`, vLLM `http://127.0.0.1:8000/v1`. The model must
support tool calls. For llama.cpp started by hand, use `openai_compat` with its
`/v1` URL, or set `runtime.llama_server_path` and let Kara manage it.

A non-local endpoint is allowed but reported plainly: `kara privacy` shows
`REMOTE` and prompts and code context are sent there. Only your user config
can set an endpoint; a repository's config cannot.

## Reasoning models

Thinking output is shown as a dim "thinking…" indicator (set
`agent.show_reasoning = true` to stream it). On slow hardware, thinking
dominates step time, so small registry models default to a per-response
budget. Override with:

```toml
[model]
reasoning = "auto"        # auto | on | off
reasoning_budget = 2048   # tokens; -1 = unlimited; 0 = registry default
```

## llama.cpp

Kara pins a llama.cpp release in [`models/runtimes.toml`](models/runtimes.toml)
with per-platform assets and SHA-256 digests: macOS (Metal), Linux (CPU,
Vulkan, CUDA, ROCm), Windows (CPU, Vulkan, CUDA). It prefers an existing
`llama-server` on PATH. To use a newer build, install it yourself or update
the pin with `scripts/update-llama-pin.sh`.
