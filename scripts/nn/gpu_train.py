#!/usr/bin/env python3
"""One-shot: download text, tokenise, train the neural re-ranker on a GPU,
export it. Pure Python + PyTorch (no Rust needed). Everything is written
inside --work (e.g. D:\\quipkeys\\work); nothing else on disk is touched.

  python scripts/nn/gpu_train.py --work D:\\quipkeys\\work

Steps (each is skipped if its output already exists, so you can re-run):
  1. download N Wiki40B shards (~74 MB each) + a few conversational corpora
  2. extract text, tokenise with QuipKeys' own vocabulary (data/nn_vocab.txt)
  3. train (prints progress; checkpoint every 500 steps; Ctrl+C is safe)
  4. export work/en_nnlm.bin  -> copy it to data/en_nnlm.bin in the repo
"""
import argparse, glob, html, io, os, re, struct, subprocess, sys, time, urllib.request, zipfile

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
WIKI = "https://storage.googleapis.com/tfds-data/datasets/wiki40b/Wiki40B.en/1.0.0/wiki40b-train.tfrecord-{:05d}-of-00128"
NLTK = "https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/{}.zip"


def log(*a):
    print(time.strftime("%H:%M:%S"), *a, flush=True)


def fetch(url, path):
    tmp = path + ".part"
    with urllib.request.urlopen(url, timeout=120) as r, open(tmp, "wb") as f:
        while True:
            b = r.read(1 << 20)
            if not b:
                break
            f.write(b)
    os.replace(tmp, path)


def wiki_text(tfrecord, out):
    cut = re.compile(r"[\x00-\x09\x0b-\x1f]")
    with open(tfrecord, "rb") as f, open(out + ".part", "w", encoding="utf-8") as o:
        while True:
            head = f.read(12)
            if len(head) < 12:
                break
            (n,) = struct.unpack("<Q", head[:8])
            s = f.read(n).decode("utf-8", "ignore")
            f.read(4)
            for chunk in s.split("_START_PARAGRAPH_\n")[1:]:
                para = cut.split(chunk.split("\n", 1)[0], 1)[0]
                for line in para.split("_NEWLINE_"):
                    if line.strip():
                        o.write(line.strip() + "\n")
    os.replace(out + ".part", out)


def chatty_text(work, out):
    """Fiction dialogue, web/chat text, reviews: the conversational side."""
    lines = []
    for c in ["gutenberg", "webtext", "nps_chat", "movie_reviews"]:
        z = os.path.join(work, c + ".zip")
        if not os.path.exists(z):
            fetch(NLTK.format(c), z)
        with zipfile.ZipFile(z) as zf:
            for name in zf.namelist():
                if name.endswith("/") or "README" in name:
                    continue
                t = zf.read(name).decode("latin-1")
                if c == "nps_chat":
                    lines += [html.unescape(m).strip() for m in re.findall(r"<Post[^>]*>(.*?)</Post>", t, re.S)]
                elif c == "webtext":
                    lines += t.splitlines()
                else:
                    lines += [" ".join(p.split()) for p in re.split(r"\n\s*\n", t)]
    with open(out, "w", encoding="utf-8") as o:
        o.write("\n".join(l for l in lines if l.strip()) + "\n")


def tokenise(files, vocab, out, repeat):
    idx = {w: i for i, w in enumerate(vocab)}
    unk, bos = len(vocab), len(vocab) + 1
    ends = set(".!?")
    buf = []
    total = 0
    with open(out + ".part", "wb") as o:
        for path in files:
            for _ in range(repeat.get(path, 1)):
                with open(path, encoding="utf-8", errors="ignore") as f:
                    for line in f:
                        buf.append(bos)
                        for raw in line.split():
                            m = re.search(r"[A-Za-z].*[A-Za-z]|[A-Za-z]", raw)
                            t = unk
                            if m:
                                core = m.group(0).lower()
                                t = idx.get(core, unk)
                                if t == unk and core.endswith("'s"):
                                    t = idx.get(core[:-2], unk)
                            buf.append(t)
                            tail = raw[m.end():] if m else raw
                            if any(ch in ends for ch in tail):
                                buf.append(bos)
                        if len(buf) > 1 << 22:
                            np.asarray(buf, dtype="<u2").tofile(o)
                            total += len(buf)
                            buf = []
            log(f"  tokenised {os.path.basename(path)} ({total + len(buf):,} tokens so far)")
        np.asarray(buf, dtype="<u2").tofile(o)
    os.replace(out + ".part", out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True, help="working folder (everything goes here)")
    ap.add_argument("--shards", type=int, default=32, help="Wiki40B shards to use (~11M words each)")
    ap.add_argument("--d", type=int, default=384)
    ap.add_argument("--layers", type=int, default=6)
    ap.add_argument("--heads", type=int, default=6)
    ap.add_argument("--ctx", type=int, default=64)
    ap.add_argument("--batch", type=int, default=64)
    ap.add_argument("--steps", type=int, default=40000)
    ap.add_argument("--minutes", type=float, default=0, help="stop training after this long")
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    # Keep temp files off C:.
    os.environ["TMP"] = os.environ["TEMP"] = os.path.join(a.work, "tmp")
    os.makedirs(os.environ["TMP"], exist_ok=True)

    import torch
    log(f"torch {torch.__version__}, CUDA available: {torch.cuda.is_available()}"
        + (f" ({torch.cuda.get_device_name(0)})" if torch.cuda.is_available() else " -- training will be very slow"))

    vocab = open(os.path.join(REPO, "data", "nn_vocab.txt"), encoding="utf-8").read().split()
    texts = []
    for i in range(a.shards):
        txt = os.path.join(a.work, f"wiki{i:03d}.txt")
        if not os.path.exists(txt):
            tf = os.path.join(a.work, f"wiki{i:03d}.tfrecord")
            log(f"downloading Wikipedia shard {i + 1}/{a.shards}")
            fetch(WIKI.format(i), tf)
            wiki_text(tf, txt)
            os.remove(tf)
        texts.append(txt)
    chatty = os.path.join(a.work, "chatty.txt")
    if not os.path.exists(chatty):
        log("downloading conversational text")
        chatty_text(a.work, chatty)
    data = os.path.join(a.work, "train.u16")
    if not os.path.exists(data):
        log("tokenising")
        tokenise([chatty] + texts, vocab, data, {chatty: 5})  # chatty first: validation (the tail) is Wikipedia
    log(f"training data: {os.path.getsize(data) // 2:,} tokens")

    cmd = [sys.executable, os.path.join(HERE, "train_lm.py"), "--data", data, "--vocab", str(len(vocab)),
           "--d", str(a.d), "--layers", str(a.layers), "--heads", str(a.heads), "--ctx", str(a.ctx),
           "--batch", str(a.batch), "--steps", str(a.steps), "--minutes", str(a.minutes),
           "--out", os.path.join(a.work, "en_nnlm.bin"), "--ckpt", os.path.join(a.work, "nnlm.pt")]
    log("training: " + " ".join(cmd[2:]))
    subprocess.run(cmd, check=True)
    log(f"done. Copy {os.path.join(a.work, 'en_nnlm.bin')} to data/en_nnlm.bin in the repo "
        "(or attach it in the chat).")


if __name__ == "__main__":
    main()
