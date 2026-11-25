"""Training data for the real-word scorer (see rw_train.py).

The scorer answers one question: a real word was typed here; given the words on
both sides, which of its look-alikes was meant (usually: the word itself)?

This script builds, from plain English text:
  * the vocabulary (most frequent tokens; words keep apostrophes, and . , ! ? ; :
    are tokens too, because "Their is" vs "There is" turns on sentence starts);
  * each word's confusion set: the hand-made list in data/en_confusions.txt plus
    every pair of vocabulary words one keyboard slip apart (adjacent-key
    substitution, a dropped, doubled or extra adjacent letter, a swap). Sets
    are symmetric and capped to the most frequent neighbours;
  * training windows: for sampled positions whose word has a confusion set,
    the CTX tokens on each side and the word itself.

    python3 -I scripts/nn/rw_data.py OUT_DIR text1.txt[:weight] text2.txt ...

Writes OUT_DIR/rw_vocab.txt, rw_table.npz, rw_windows.npz. Lines (paragraphs)
are independent: context never crosses a line.
"""
import collections
import re
import sys

import numpy as np

VOCAB = 20000
CTX = 5
MAX_NEIGHBOURS = 12
MEANT_VOCAB = 15000  # only this many most frequent words can be "what was meant"
TARGET_PER_WORD = 4000  # sampling: frequent words are thinned towards this many examples
PAD, UNK = 0, 1
SPECIALS = ["<pad>", "<unk>"]

# Window positions around the word (the word itself is left out).
OFFSETS = np.array([k for k in range(2 * CTX + 1) if k != CTX])

TOKEN = re.compile(r"[a-z]+(?:'[a-z]+)?|[.,!?;:]")

ROWS = ["qwertyuiop", "asdfghjkl", "zxcvbnm"]
POS = {c: (r, i + (0.5 * r if r else 0)) for r, row in enumerate(ROWS) for i, c in enumerate(row)}


def adjacent(a, b):
    if a not in POS or b not in POS:
        return False
    (r1, x1), (r2, x2) = POS[a], POS[b]
    return a != b and abs(r1 - r2) <= 1 and abs(x1 - x2) <= 1.0


# Relation codes, shared with the Rust side (src/rwscorer.rs).
REL_SELF, REL_LIST, REL_SUB, REL_DROP, REL_EXTRA, REL_SWAP = 0, 1, 2, 3, 4, 5
NREL = 6


def slip(typed, meant):
    """How `typed` could be a one-slip version of `meant`, or None."""
    if typed == meant:
        return REL_SELF
    lt, lm = len(typed), len(meant)
    if lt == lm:
        diff = [i for i in range(lt) if typed[i] != meant[i]]
        if len(diff) == 1 and adjacent(typed[diff[0]], meant[diff[0]]):
            return REL_SUB
        if len(diff) == 2 and diff[1] == diff[0] + 1 and typed[diff[0]] == meant[diff[1]] and typed[diff[1]] == meant[diff[0]]:
            return REL_SWAP
        return None
    if lt == lm - 1:  # a letter dropped
        for i in range(lm):
            if meant[:i] + meant[i + 1:] == typed:
                return REL_DROP
        return None
    if lt == lm + 1:  # an extra letter: doubled, or next to its neighbour
        for i in range(lt):
            if typed[:i] + typed[i + 1:] == meant:
                extra = typed[i]
                near = [typed[j] for j in (i - 1, i + 1) if 0 <= j < lt]
                if any(extra == n or adjacent(extra, n) for n in near):
                    return REL_EXTRA
        return None
    return None


def read_lines(path):
    with open(path, encoding="utf-8", errors="ignore") as f:
        for line in f:
            yield line.lower().replace("’", "'")


def main():
    out = sys.argv[1]
    sources = []
    for a in sys.argv[2:]:
        p, _, w = a.partition(":")
        sources.append((p, int(w or 1)))

    print("counting tokens...", flush=True)
    counts = collections.Counter()
    for p, w in sources:
        for line in read_lines(p):
            for t in TOKEN.findall(line):
                counts[t] += w
    words = [t for t, _ in counts.most_common(VOCAB)]
    # Every listed confusion word gets a place, even if rare in this text.
    listed = []
    for line in open("data/en_confusions.txt", encoding="utf-8"):
        if line.startswith("#") or not line.split():
            continue
        listed.append(line.split())
    have = set(words)
    for s in listed:
        for t in s:
            if t not in have:
                words.append(t)
                have.add(t)
    itos = SPECIALS + words
    stoi = {t: i for i, t in enumerate(itos)}
    V = len(itos)
    freq = np.array([0, 0] + [counts.get(t, 0) for t in words], dtype=np.float64)
    print(f"vocab {V}", flush=True)

    # Confusion sets: for each typed word, the words it could be a slip of.
    # Names (words almost always capitalised: data/en_capitalized.txt) and
    # very rare words are never offered as what was meant.
    names = set()
    for line in open("data/en_capitalized.txt", encoding="utf-8"):
        if not line.startswith("#") and line.split():
            names.add(line.split()[0])
    listed_words = {t for s_ in listed for t in s_}
    have = {t for t in words[:MEANT_VOCAB] if t not in names} | listed_words
    letters = "abcdefghijklmnopqrstuvwxyz"
    meant_of = collections.defaultdict(dict)  # typed -> {meant: rel}
    for t in words:
        if not re.fullmatch(r"[a-z]+(?:'[a-z]+)?", t) or len(t) < 2:
            continue
        n = len(t)
        for i in range(n):  # adjacent-key substitution
            for c in letters:
                if adjacent(t[i], c):
                    u = t[:i] + c + t[i + 1:]
                    if u in have:
                        meant_of[t][u] = REL_SUB
        for i in range(n - 1):  # two letters swapped
            u = t[:i] + t[i + 1] + t[i] + t[i + 2:]
            if u != t and u in have:
                meant_of[t][u] = REL_SWAP
        for i in range(n + 1):  # a letter dropped: the meant word has one more
            for c in letters + "'":
                u = t[:i] + c + t[i:]
                if u in have:
                    meant_of[t].setdefault(u, REL_DROP)
        for i in range(n):  # an extra letter: doubled, or next to a neighbour
            u = t[:i] + t[i + 1:]
            near = [t[j] for j in (i - 1, i + 1) if 0 <= j < n]
            if len(u) >= 1 and u in have and any(t[i] == x or adjacent(t[i], x) for x in near):
                meant_of[t].setdefault(u, REL_EXTRA)
    for s_ in listed:
        for a_ in s_:
            for b_ in s_:
                if a_ != b_:
                    meant_of[a_][b_] = REL_LIST
    K = MAX_NEIGHBOURS + 1
    table = np.zeros((V, K), dtype=np.int32)  # slot 0: the word itself
    rel = np.zeros((V, K), dtype=np.int8)
    count = np.ones(V, dtype=np.int32)
    table[:, 0] = np.arange(V)
    for t, d in meant_of.items():
        i = stoi[t]
        keep = sorted(d, key=lambda u: (d[u] != REL_LIST, -counts.get(u, 0)))[:MAX_NEIGHBOURS]
        for u in keep:
            table[i, count[i]] = stoi[u]
            rel[i, count[i]] = d[u]
            count[i] += 1
    # Reverse (for training): the typed words a meant word can turn into.
    rev = collections.defaultdict(list)
    for i in range(V):
        for k in range(1, count[i]):
            rev[int(table[i, k])].append(i)
    R = 16
    rtable = np.zeros((V, R), dtype=np.int32)
    rcount = np.zeros(V, dtype=np.int32)
    for m, ts in rev.items():
        ts = sorted(ts, key=lambda t: -freq[t])[:R]
        rtable[m, :len(ts)] = ts
        rcount[m] = len(ts)
    conf = rcount > 0
    print(f"words that can be mistyped as another word: {int(conf.sum())}; typed words with look-alikes: {int((count > 1).sum())}", flush=True)

    with open(f"{out}/rw_vocab.txt", "w", encoding="utf-8") as f:
        f.write("\n".join(itos) + "\n")
    np.savez(f"{out}/rw_table.npz", table=table, rel=rel, count=count, freq=freq, rtable=rtable, rcount=rcount)

    # Windows.
    print("cutting windows...", flush=True)
    rng = np.random.default_rng(1)
    keep_p = np.minimum(1.0, TARGET_PER_WORD / np.maximum(freq, 1.0)) ** 0.5
    keep_p[~conf] = 0.0  # only words that can be mistyped as another word
    wins, labels = [], []
    for p, w in sources:
        buf = []
        for line in read_lines(p):
            ids = [stoi.get(t, UNK) for t in TOKEN.findall(line)]
            if len(ids) < 3:
                continue
            buf.append(np.array(ids, dtype=np.int32))
        for _ in range(w):
            for ids in buf:
                pk = keep_p[ids]
                sel = np.nonzero(rng.random(len(ids)) < pk)[0]
                if not len(sel):
                    continue
                padded = np.concatenate([np.zeros(CTX, np.int32), ids, np.zeros(CTX, np.int32)])
                wins.append(padded[sel[:, None] + OFFSETS])
                labels.append(ids[sel])
        print(f"  {p}: {sum(len(l) for l in labels)} windows so far", flush=True)
    wins = np.concatenate(wins).astype(np.uint16)
    labels = np.concatenate(labels).astype(np.uint16)
    np.savez(f"{out}/rw_windows.npz", ctx=wins, label=labels)
    print(f"{len(labels)} windows", flush=True)


if __name__ == "__main__":
    main()
