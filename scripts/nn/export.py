#!/usr/bin/env python3
"""Export a train_lm.py checkpoint to the engine format, and print reference
logits so the Rust inference can be checked against PyTorch.

  python3 scripts/nn/export.py --ckpt nnlm.pt --vocab 32768 --d 128 --layers 2 --heads 4 --ctx 32 --out data/en_nnlm.bin
"""
import argparse, sys, os
sys.path.insert(0, os.path.dirname(__file__))
import torch
from train_lm import WordLM, export

ap = argparse.ArgumentParser()
for k, d in [("vocab", 32768), ("d", 128), ("layers", 2), ("heads", 4), ("ctx", 32)]:
    ap.add_argument("--" + k, type=int, default=d)
ap.add_argument("--ckpt", required=True)
ap.add_argument("--out", default="data/en_nnlm.bin")
ap.add_argument("--check", default="32769,10,2,5,100")  # bos + a few ids
args = ap.parse_args()
m = WordLM(args.vocab + 2, args.d, args.layers, args.heads, args.ctx)
m.load_state_dict(torch.load(args.ckpt, map_location="cpu"))
m.eval()
export(m, args.out, args)
ids = [int(x) for x in args.check.split(",")]
with torch.no_grad():
    lg = m(torch.tensor([ids]))[0, -1]
print("reference logits after", ids, "for ids 0..4:", " ".join(f"{v:.4f}" for v in lg[:5].tolist()))
