"""Train the real-word scorer and export it for the engine (src/rwscorer.rs).

Given the 5 tokens before and after a typed real word, score each of its
look-alikes (rw_data.py) as what was meant:

    score(c) = E[c] . h(context) + bias[c] + rel_weight[how c relates to typed]
               + keep[typed] if c is the typed word itself

h(context) is a two-layer network over the 10 context embeddings. Training
takes real sentences, swaps the word for a plausible slip half the time, and
learns to pick the word that was really there: so it learns when to leave a
word alone as much as when to change it.

    python3 scripts/nn/rw_train.py DATA_DIR OUT.bin [--epochs 2] [--steps N]
"""
import argparse
import math
import struct
import time

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

NREL = 6
CTX = 5


class Scorer(nn.Module):
    def __init__(self, V, d=64, hidden=256):
        super().__init__()
        self.emb = nn.Embedding(V, d, padding_idx=0)
        self.l1 = nn.Linear(2 * CTX * d, hidden)
        self.l2 = nn.Linear(hidden, d)
        self.bias = nn.Embedding(V, 1)
        self.keep = nn.Embedding(V, 1)
        self.rel = nn.Parameter(torch.zeros(NREL))
        nn.init.normal_(self.emb.weight, std=0.1)
        nn.init.zeros_(self.bias.weight)
        nn.init.zeros_(self.keep.weight)

    def forward(self, ctx, typed, cands, crel, cmask):
        B = ctx.shape[0]
        h = self.l2(F.gelu(self.l1(self.emb(ctx).view(B, -1))))
        s = (self.emb(cands) * h[:, None, :]).sum(-1) + self.bias(cands).squeeze(-1) + self.rel[crel.long()]
        s[:, 0] = s[:, 0] + self.keep(typed).squeeze(-1)
        return s.masked_fill(~cmask, -1e9)


def shorten(rng, ctx, p_right=0.7, p_left=0.2):
    """In use, the word is judged while the sentence is still being typed:
    often only a word or two follows it, and the box may have started just
    before it. Train on the same: cut the right context to 1-5 tokens most of
    the time, and sometimes the left."""
    ctx = ctx.copy()
    n = len(ctx)
    keep_r = rng.integers(1, CTX + 1, n)
    cut_r = rng.random(n) < p_right
    pos = np.arange(CTX)[None, :]
    ctx[:, CTX:][cut_r[:, None] & (pos >= keep_r[:, None])] = 0
    keep_l = rng.integers(0, CTX, n)
    cut_l = rng.random(n) < p_left
    ctx[:, :CTX][cut_l[:, None] & (pos < (CTX - keep_l)[:, None])] = 0
    return ctx


def make_batch(rng, ctx, label, T, rho, short=True):
    """Swap the word for a slip with probability rho; return model inputs and targets."""
    if short:
        ctx = shorten(rng, ctx)
    w = label.astype(np.int64)
    can = T["rcount"][w] > 0
    flip = (rng.random(len(w)) < rho) & can
    typed = w.copy()
    pick = (rng.random(len(w)) * np.maximum(T["rcount"][w], 1)).astype(np.int64)
    typed[flip] = T["rtable"][w[flip], pick[flip]]
    cands = T["table"][typed]
    crel = T["rel"][typed]
    cmask = np.arange(cands.shape[1])[None, :] < T["count"][typed][:, None]
    hit = (cands == w[:, None]) & cmask
    ok = hit.any(1)
    target = hit.argmax(1)
    return (
        torch.from_numpy(ctx.astype(np.int64)[ok]),
        torch.from_numpy(typed[ok]),
        torch.from_numpy(cands[ok].astype(np.int64)),
        torch.from_numpy(crel[ok].astype(np.int64)),
        torch.from_numpy(cmask[ok]),
        torch.from_numpy(target[ok]),
        torch.from_numpy(flip[ok]),
    )


def evaluate(model, ctx, label, T, rho=0.5, seed=7, short=False):
    rng = np.random.default_rng(seed)
    model.eval()
    fixed = n_flip = kept = n_clean = 0
    with torch.no_grad():
        for i in range(0, len(label), 8192):
            c, t, k, r, m, y, f = make_batch(rng, ctx[i:i + 8192], label[i:i + 8192], T, rho, short)
            pred = model(c, t, k, r, m).argmax(1)
            right = pred == y
            fixed += int(right[f].sum())
            n_flip += int(f.sum())
            kept += int(right[~f].sum())
            n_clean += int((~f).sum())
    model.train()
    return fixed / max(n_flip, 1), kept / max(n_clean, 1)


def export(model, vocab, T, path):
    """Little-endian binary: header, vocab, int8 embeddings (per-row scale),
    f32 layers, biases, relation weights, candidate table."""
    E = model.emb.weight.detach().numpy().astype(np.float32)
    V, d = E.shape
    H = model.l1.out_features
    scale = np.maximum(np.abs(E).max(1), 1e-8) / 127.0
    q = np.clip(np.round(E / scale[:, None]), -127, 127).astype(np.int8)
    K = T["table"].shape[1]
    with open(path, "wb") as f:
        f.write(b"QKRW1\0\0\0")
        f.write(struct.pack("<6I", V, d, CTX, H, K, NREL))
        for w in vocab:
            b = w.encode("utf-8")
            f.write(struct.pack("<B", len(b)))
            f.write(b)
        f.write(scale.astype("<f4").tobytes())
        f.write(q.tobytes())
        for p in (model.l1.weight, model.l1.bias, model.l2.weight, model.l2.bias):
            f.write(p.detach().numpy().astype("<f4").tobytes())
        f.write(model.bias.weight.detach().numpy().reshape(-1).astype("<f4").tobytes())
        f.write(model.keep.weight.detach().numpy().reshape(-1).astype("<f4").tobytes())
        f.write(model.rel.detach().numpy().astype("<f4").tobytes())
        f.write(T["count"].astype(np.uint8).tobytes())
        f.write(T["table"].astype("<u2").tobytes())
        f.write(T["rel"].astype(np.uint8).tobytes())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("data")
    ap.add_argument("out")
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--batch", type=int, default=2048)
    ap.add_argument("--lr", type=float, default=3e-3)
    ap.add_argument("--rho", type=float, default=0.5)
    ap.add_argument("--d", type=int, default=64)
    ap.add_argument("--hidden", type=int, default=256)
    ap.add_argument("--threads", type=int, default=4)
    args = ap.parse_args()
    torch.set_num_threads(args.threads)
    torch.manual_seed(0)
    vocab = open(f"{args.data}/rw_vocab.txt", encoding="utf-8").read().rstrip("\n").split("\n")
    T = dict(np.load(f"{args.data}/rw_table.npz"))
    W = np.load(f"{args.data}/rw_windows.npz")
    ctx, label = W["ctx"], W["label"]
    rng = np.random.default_rng(0)
    perm = rng.permutation(len(label))
    nval = min(200_000, len(label) // 50)
    val, tr = perm[:nval], perm[nval:]
    vctx, vlab = ctx[val], label[val]
    model = Scorer(len(vocab), args.d, args.hidden)
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.0)
    steps = int(args.epochs * len(tr) / args.batch)
    sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: min(1.0, s / 500) * 0.5 * (1 + math.cos(math.pi * min(s, steps) / steps)))
    print(f"{len(vocab)} words, {len(tr)} training windows, {steps} steps", flush=True)
    t0 = time.time()
    step = 0
    while step < steps:
        order = rng.permutation(tr)
        for i in range(0, len(order) - args.batch + 1, args.batch):
            idx = np.sort(order[i:i + args.batch])
            c, t, k, r, m, y, _ = make_batch(rng, ctx[idx], label[idx], T, args.rho)
            loss = F.cross_entropy(model(c, t, k, r, m), y)
            opt.zero_grad()
            loss.backward()
            opt.step()
            sched.step()
            step += 1
            if step % 1000 == 0 or step == steps:
                fix, keep = evaluate(model, vctx[:50_000], vlab[:50_000], T)
                sfix, skeep = evaluate(model, vctx[:50_000], vlab[:50_000], T, short=True)
                print(f"step {step}/{steps}  loss {loss.item():.3f}  val full context: fixed {fix:.3f} kept {keep:.3f}; "
                      f"short: fixed {sfix:.3f} kept {skeep:.3f}  ({time.time() - t0:.0f}s)", flush=True)
                export(model, vocab, T, args.out)
            if step >= steps:
                break
    fix, keep = evaluate(model, vctx, vlab, T)
    print(f"final val: slips fixed {fix:.4f}, clean kept {keep:.4f}", flush=True)
    export(model, vocab, T, args.out)
    torch.save(model.state_dict(), args.out + ".pt")


if __name__ == "__main__":
    main()
