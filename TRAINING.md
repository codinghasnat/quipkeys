# Training the neural re-ranker on a GPU

## Quick start (Windows PC with an NVIDIA GPU, everything on D:)

```powershell
D:
git clone https://github.com/codinghasnat/quipkeys D:\quipkeys\repo
cd D:\quipkeys\repo; git checkout ccr-9d6287c2-w4on6w
# use an env that already has PyTorch with CUDA (check: prints "True")
conda run -n whisperkeys python -c "import torch; print(torch.__version__, torch.cuda.is_available())"
conda run -n whisperkeys --no-capture-output python scripts\nn\gpu_train.py --work D:\quipkeys\work
```

It downloads about 2.5 GB into `D:\quipkeys\work`, prints progress, and after about 4–8 hours writes
`D:\quipkeys\work\en_nnlm.bin` (about 25 MB). Put that file in `data\en_nnlm.bin` and push, or attach it in
the chat. Re-running resumes from whatever is already downloaded or tokenised.


QuipKeys can use a small neural word language model to break ties between candidate
words when the n-gram context isn't enough (`I wnt home` vs `I wnt a coffee`). The engine
runs it in pure Rust on the CPU. Training needs a GPU: an RTX 2060 (6 GB) is plenty,
and so is any GCP GPU (a T4 or L4 is the cheap option).

## 1. Get the text (any machine)

```sh
scripts/fetch_data.sh                  # or just the Wikipedia/NLTK part of it
cargo build --release
# word ids, using exactly the engine's dictionary and tokenisation:
./target/release/qk-tokenize train.u16 32768 /path/to/wiki/*.txt /path/to/chatty.txt
```

Each Wiki40B file is about 11M words; all 128 are about 1.4B words (about 2.8 GB as `train.u16`).

## 2. Train

```sh
pip install torch numpy
python3 scripts/nn/train_lm.py --data train.u16 --vocab 32768 \
    --d 384 --layers 6 --heads 6 --ctx 64 --batch 96 --steps 60000 \
    --out data/en_nnlm.bin --ckpt nnlm.pt
```

| Setup | Model | Time (rough) |
|---|---|---|
| RTX 2060 6 GB | d=384, 6 layers (about 23M params) | about 1–2 h for 40k steps, including download (add `--steps 80000` for a better model) |
| GCP L4 / A100 | d=512, 8 layers, `--batch 256` | 2–4 h |
| CPU (proof of concept) | d=128, 2 layers | 1 h, weak model |

It prints validation perplexity every 500 steps and saves `nnlm.pt`. Interrupt it at any
time and export the checkpoint:

```sh
python3 scripts/nn/export.py --ckpt nnlm.pt --vocab 32768 --d 384 --layers 6 --heads 6 --ctx 64 --out data/en_nnlm.bin
```

## 3. Use it

Rebuild (`cargo build --release`; the model is compiled in), then measure:

```sh
./target/release/qk-bench --level all --configs full,nn
```

If `nn` beats `full`, set `nn_weight` in `src/decoder.rs` to the best value. The engine's
inference cost scales with d² × layers: d=384 × 6 layers is about 1 ms per word on a laptop CPU.
