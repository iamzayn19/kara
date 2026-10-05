# Inference

Kara Core (agent, tools, context, permissions) is the same everywhere. Where
inference runs is a configuration choice, behind one interface
(`InferenceProvider` in `kara-inference`). Nothing else in Kara knows or
cares whether a model runs on this machine, on another machine, or behind an
HTTP endpoint.

| Provider | `inference.provider` | Where it runs |
|---|---|---|
| Local runtime | `local` (default) | This machine, through a runtime Kara manages (llama.cpp today). Optional. |
| Another Kara machine | `kara` | A machine you own running `kara serve --inference`. |
| OpenAI-compatible endpoint | `openai_compat`, `ollama`, `lmstudio`, `vllm` | Any compatible server you control: Ollama, LM Studio, vLLM, a llama.cpp server, and others. |

Kara never uses a Kara-hosted service and has no paid inference dependency.

## Hardware-adaptive behaviour

There is no minimum machine specification for Kara. On start-up the local
backend detects RAM, CPU architecture, accelerators (Metal, CUDA, ROCm,
Vulkan) and disk space, then:

* if a useful local model fits, it can be offered, with download size,
  memory needs, license and source shown first, and downloaded only after
  confirmation;
* if nothing useful fits, Kara says so and recommends compute you own
  elsewhere. Everything that does not need a model (search, `/matrix`,
  `/diff`, `/test`, `/undo`, git inspection) keeps working.

Installing the CLI or the VS Code extension never downloads a model.

## Another machine you own

On the machine with the compute:

```sh
kara serve --inference                        # loopback only (127.0.0.1:7878)
kara serve --inference --listen 0.0.0.0:7878  # accept other machines
```

It resolves that machine's own inference (local runtime or endpoint), prints
a token, and exposes an OpenAI-compatible API (`/v1/chat/completions`,
`/v1/models`) that requires `Authorization: Bearer <token>`. The token is
generated once and kept in an owner-only file in Kara's config directory.

On your other machine:

```sh
kara connect http://gpu-box:7878 --token <token>
```

`kara connect` checks the server and the token, stores the token in an
owner-only file, and sets `inference.provider = "kara"` and
`inference.endpoint`. `kara privacy` then reports that prompts and code
context go to that machine.

Traffic is plain HTTP. Use it on a network you trust (home or office LAN, a
VPN such as WireGuard or Tailscale) or through an SSH tunnel:

```sh
ssh -L 7878:127.0.0.1:7878 gpu-box   # then: kara connect http://127.0.0.1:7878
```

## OpenAI-compatible endpoints

```sh
kara config set inference.provider ollama        # default http://127.0.0.1:11434/v1
kara config set inference.model qwen3:8b         # empty = first model the server lists
kara config set inference.provider openai_compat
kara config set inference.endpoint http://10.0.0.5:8000/v1
kara config set inference.api_key_env MY_TOKEN   # if the server needs a bearer token
```

Default endpoints: Ollama `http://127.0.0.1:11434/v1`, LM Studio
`http://127.0.0.1:1234/v1`, vLLM `http://127.0.0.1:8000/v1`. The model must
support tool calling. Only your user configuration can choose the provider
or endpoint; a repository's `.kara/config.toml` cannot.

## Local models (optional)

Downloadable models are data in [`models/registry.toml`](models/registry.toml).
Each entry pins a Hugging Face repository, a commit revision, a file, its size
and SHA-256, the license, and the context and memory figures used for
planning. Add or override entries in `models.toml` in Kara's config directory.

| Tier | Model | Quantization | Download | Notes |
|---|---|---|---|---|
| high | Qwen3.6-35B-A3B | Q4_K_M | 20.4 GB | Mixture of experts, about 3B active parameters; experts can run from system RAM on smaller GPUs. |
| mid | Qwen3.6-27B | Q4_K_M | 19.1 GB | Dense; needs the whole model in fast memory. |
| mid-low | Qwen3-14B | Q4_K_M | 9.0 GB | |
| low | Qwen3-4B | Q4_K_M | 2.5 GB | Runs on 8-16 GB machines; thinking capped at 1024 tokens per step. |
| minimal | Qwen3-1.7B | Q4_K_M | 1.3 GB | Not auto-selected; for smoke tests. |
| mid | Qwen3.8-27B | Q4_K_M | 19.0 GB | Newer release; manual selection only until evaluated. |

All listed models are Apache-2.0. Kara never bundles model weights. Downloads
resume after interruption and are verified against the pinned SHA-256. The
CLI and the VS Code extension share the same downloaded models.

```sh
kara models                       # what fits this machine
kara models pull qwen3-4b-q4_k_m  # shows size, memory, license, source; asks first
kara models use qwen3-4b-q4_k_m   # or: kara models auto
kara models verify qwen3-4b-q4_k_m
```

### Automatic selection

1. Detect hardware and compute a fast-memory budget: about 70% of RAM on
   Apple Silicon, about 92% of VRAM on discrete GPUs, otherwise a share of RAM
   for CPU inference.
2. For each eligible model (permitted license, tool calling,
   `auto_select = true`), estimate weights + KV cache + overhead at its
   default context, halving the context down to its minimum before giving up.
   MoE models may instead run with experts in system RAM on a discrete GPU.
3. Pick the highest *measured* Kara eval score when every fitting model has
   one; otherwise rank all of them by the provisional `quality_rank`, so one
   measured model never outranks unmeasured ones. A model that fits only
   with offloading ranks below one that fits fully.
4. If nothing fits, recommend another machine or an endpoint instead.

Larger is not assumed to be better; see
[docs/MODEL_EVALUATION.md](docs/MODEL_EVALUATION.md). Memory figures
(`kv_bytes_per_token`) are estimates; if a model fails to load, lower
`inference.context_length` or pick a smaller model.

### Local runtime

The local backend pins a llama.cpp release in
[`models/runtimes.toml`](models/runtimes.toml) with per-platform assets and
SHA-256 digests (macOS Metal; Linux CPU, Vulkan, CUDA, ROCm; Windows CPU,
Vulkan, CUDA). It prefers a `llama-server` already on PATH, installs the
pinned build only after asking, binds it to `127.0.0.1`, and stops it when
Kara exits (servers orphaned by a killed Kara are stopped on the next start).
Settings live under `[inference.local]`.

## Reasoning models

Thinking output shows as a dim "thinking…" line (set
`agent.show_reasoning = true` to stream it). Small registry models default to
a thinking budget per response. Override with:

```toml
[inference]
reasoning = "auto"        # auto | on | off
reasoning_budget = 2048   # tokens; -1 = unlimited; 0 = registry default
```
