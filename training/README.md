# Optional training tools

**Nothing here is required to use Kara.** Kara v0.1 uses existing
open-weight models. This directory prepares for an optional, future
Kara-tuned model and for local LoRA experiments on small models.

## Data rules

Training data may only come from:

- permissively licensed public datasets,
- permissively licensed source repositories where legally appropriate,
- synthetic data whose use for training is permitted,
- data created by the project owner,
- Kara traces that a user explicitly opted into (`privacy.training_data = true`
  in their own `~/.kara/config.toml`).

Never use private code without the owner's explicit permission. Never use
outputs of proprietary models unless their terms explicitly allow training on
them. Kara never uploads traces: they stay in `~/.kara/traces/` until you
move them yourself.

## Pipeline

```
~/.kara/traces/*.jsonl  (opt-in, local)
        │  sanitize_traces.py      redact secrets, drop failures and private paths
        ▼
dataset/*.jsonl          (schema: dataset_schema.json)
        │  eval_to_dataset.py      add solved fixture tasks from `kara eval`
        ▼
check_hardware.py        refuses configurations this machine cannot train
        │
lora_train.py            QLoRA on a small model (transformers + peft)
        │
merge_adapter.py         merge the adapter, then convert to GGUF with llama.cpp
```

```sh
python3 -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt          # only for training; not for Kara
python3 sanitize_traces.py --out dataset/traces.jsonl
python3 eval_to_dataset.py ../tests/evals/results/*.json --suite ../tests/fixtures/repos --out dataset/eval.jsonl
python3 check_hardware.py --model Qwen/Qwen3-1.7B --method qlora
python3 lora_train.py --model Qwen/Qwen3-1.7B --data dataset/ --out adapters/qwen3-1.7b-kara
```

If `check_hardware.py` reports `Hardware insufficient for this training
configuration.`, do not try anyway: pick a smaller model or a machine with a
supported GPU. The scripts do not pretend to train what the hardware cannot.

## Tests

`python3 -m unittest discover tests` (standard library only).
