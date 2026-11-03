#!/usr/bin/env python3
"""Train QuipKeys' small neural word language model and export it for the engine.

The model is a tiny GPT-style transformer over the engine's own word ids
(produced by `qk-tokenize`). The engine only uses it to re-rank a handful of
candidate words when the n-gram model is unsure, so it can stay small.

Runs on CPU (slow, proof-of-concept) or any CUDA GPU (RTX 2060 6 GB is plenty;
GCP works too). Typical runs:

  # 1. tokenise (in the repo, after `cargo build --release`):
  ./target/release/qk-tokenize train.u16 32768 wiki/*.txt chatty.txt
  # 2a. proof of concept on CPU (~1 hour):
  python3 scripts/nn/train_lm.py --data train.u16 --vocab 32768 --d 128 --layers 2 --heads 4 --ctx 32 --steps 3000 --batch 32
  # 2b. the real thing on an RTX 2060 (~4-8 hours):
  python3 scripts/nn/train_lm.py --data train.u16 --vocab 32768 --d 384 --layers 6 --heads 6 --ctx 64 --steps 60000 --batch 96
  # 3. it writes data/en_nnlm.bin (int8); rebuild the engine to pick it up.

Export format (little-endian): b"QKNN1\\n", u32 vocab_total, d, layers, heads, ctx,
then tensors in model order. Matrices are int8 with one f32 scale per output
row; vectors (biases, LayerNorm) are f32.
"""
import argparse, math, os, struct, time

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F


class Block(nn.Module):
    def __init__(self, d, heads):
        super().__init__()
        self.ln1 = nn.LayerNorm(d)
        self.qkv = nn.Linear(d, 3 * d)
        self.proj = nn.Linear(d, d)
        self.ln2 = nn.LayerNorm(d)
        self.fc1 = nn.Linear(d, 4 * d)
        self.fc2 = nn.Linear(4 * d, d)
        self.heads = heads

    def forward(self, x):
        B, T, D = x.shape
        q, k, v = self.qkv(self.ln1(x)).split(D, dim=2)
        sh = lambda t: t.view(B, T, self.heads, D // self.heads).transpose(1, 2)
        a = F.scaled_dot_product_attention(sh(q), sh(k), sh(v), is_causal=True)
        x = x + self.proj(a.transpose(1, 2).reshape(B, T, D))
        return x + self.fc2(F.gelu(self.fc1(self.ln2(x)), approximate="tanh"))


class WordLM(nn.Module):
    def __init__(self, vocab, d, layers, heads, ctx):
        super().__init__()
        self.tok = nn.Embedding(vocab, d)
        self.pos = nn.Embedding(ctx, d)
        self.blocks = nn.ModuleList([Block(d, heads) for _ in range(layers)])
        self.ln_f = nn.LayerNorm(d)
        self.ctx = ctx
        nn.init.normal_(self.tok.weight, std=0.02)
        nn.init.normal_(self.pos.weight, std=0.01)

    def forward(self, idx):
        x = self.tok(idx) + self.pos(torch.arange(idx.shape[1], device=idx.device))
        for b in self.blocks:
            x = b(x)
        return self.ln_f(x) @ self.tok.weight.T  # tied output embedding


def export(model, path, args):
    def q8(w):
        w = w.detach().float().cpu().numpy()
        scale = np.maximum(np.abs(w).max(axis=1), 1e-8) / 127.0
        q = np.clip(np.round(w / scale[:, None]), -127, 127).astype(np.int8)
        return scale.astype("<f4").tobytes() + q.tobytes()

    def f32(v):
        return v.detach().float().cpu().numpy().astype("<f4").tobytes()

    vt = args.vocab + 2
    with open(path, "wb") as f:
        f.write(b"QKNN1\n" + struct.pack("<5I", vt, args.d, args.layers, args.heads, args.ctx))
        f.write(q8(model.tok.weight))
        f.write(f32(model.pos.weight))
        for b in model.blocks:
            f.write(f32(b.ln1.weight) + f32(b.ln1.bias))
            f.write(q8(b.qkv.weight) + f32(b.qkv.bias))
            f.write(q8(b.proj.weight) + f32(b.proj.bias))
            f.write(f32(b.ln2.weight) + f32(b.ln2.bias))
            f.write(q8(b.fc1.weight) + f32(b.fc1.bias))
            f.write(q8(b.fc2.weight) + f32(b.fc2.bias))
        f.write(f32(model.ln_f.weight) + f32(model.ln_f.bias))
    print(f"exported {path} ({os.path.getsize(path) / 1e6:.1f} MB)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--vocab", type=int, default=32768)
    ap.add_argument("--d", type=int, default=128)
    ap.add_argument("--layers", type=int, default=2)
    ap.add_argument("--heads", type=int, default=4)
    ap.add_argument("--ctx", type=int, default=32)
    ap.add_argument("--batch", type=int, default=32)
    ap.add_argument("--steps", type=int, default=3000)
    ap.add_argument("--lr", type=float, default=3e-3)
    ap.add_argument("--minutes", type=float, default=0, help="stop after this long (0 = no limit)")
    ap.add_argument("--out", default="data/en_nnlm.bin")
    ap.add_argument("--ckpt", default="nnlm.pt")
    args = ap.parse_args()

    dev = "cuda" if torch.cuda.is_available() else "cpu"
    data = np.memmap(args.data, dtype="<u2", mode="r")
    n_val = min(len(data) // 50, 2_000_000)
    train, val = data[:-n_val], data[-n_val:]
    print(f"{len(data):,} tokens ({len(train):,} train), device {dev}")
    model = WordLM(args.vocab + 2, args.d, args.layers, args.heads, args.ctx).to(dev)
    print(f"{sum(p.numel() for p in model.parameters()) / 1e6:.1f}M parameters")
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.01, betas=(0.9, 0.98))
    amp = torch.autocast("cuda", dtype=torch.float16) if dev == "cuda" else torch.autocast("cpu", enabled=False)
    try:
        scaler = torch.amp.GradScaler(enabled=dev == "cuda")
    except (AttributeError, TypeError):  # older PyTorch
        scaler = torch.cuda.amp.GradScaler(enabled=dev == "cuda")
    rng = np.random.default_rng(0)

    def batch(src):
        ix = rng.integers(0, len(src) - args.ctx - 1, args.batch)
        x = np.stack([src[i:i + args.ctx + 1] for i in ix]).astype(np.int64)
        x = torch.from_numpy(x).to(dev)
        return x[:, :-1], x[:, 1:]

    def evaluate():
        model.eval()
        with torch.no_grad(), amp:
            losses = [F.cross_entropy(model(x).float().flatten(0, 1), y.flatten()).item() for x, y in (batch(val) for _ in range(20))]
        model.train()
        return sum(losses) / len(losses)

    t0 = time.time()
    for step in range(1, args.steps + 1):
        lr = args.lr * min(1.0, step / 200) * 0.5 * (1 + math.cos(math.pi * step / args.steps))
        for g in opt.param_groups:
            g["lr"] = lr
        x, y = batch(train)
        with amp:
            loss = F.cross_entropy(model(x).float().flatten(0, 1), y.flatten())
        opt.zero_grad(set_to_none=True)
        scaler.scale(loss).backward()
        scaler.unscale_(opt)
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0)
        scaler.step(opt)
        scaler.update()
        if step % 100 == 0 or step == args.steps:
            el = time.time() - t0
            tps = step * args.batch * args.ctx / el
            msg = f"step {step} loss {loss.item():.3f} lr {lr:.2e} {tps:,.0f} tok/s"
            if step % 500 == 0 or step == args.steps:
                v = evaluate()
                msg += f" | val loss {v:.3f} ppl {math.exp(v):.0f}"
                torch.save(model.state_dict(), args.ckpt)
            print(msg, flush=True)
            if args.minutes and el > args.minutes * 60:
                print("time limit reached")
                break
    print(f"final val loss {evaluate():.3f}")
    export(model, args.out, args)


if __name__ == "__main__":
    main()
