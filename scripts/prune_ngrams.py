"""Write a smaller copy of an n-gram counts file (src/ngrams.rs format) by
dropping word pairs seen fewer than MIN2 times and triples seen fewer than MIN3.

    python3 -I scripts/prune_ngrams.py data/en_ngrams.bin out.bin MIN2 MIN3
"""
import struct
import sys


def main():
    src, dst, min2, min3 = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    b = open(src, "rb").read()
    magic = b[:6]
    assert magic in (b"QKNG1\n", b"QKNG2\n")
    p = 6

    def u32():
        nonlocal p
        v = struct.unpack_from("<I", b, p)[0]
        p += 4
        return v

    def var():
        nonlocal p
        x = s = 0
        while True:
            c = b[p]
            p += 1
            x |= (c & 0x7F) << s
            s += 7
            if c < 0x80:
                return x

    out = bytearray(magic)

    def put(x):
        while True:
            c = x & 0x7F
            x >>= 7
            if x:
                out.append(c | 0x80)
            else:
                out.append(c)
                return

    V = u32()
    out += struct.pack("<I", V)
    start = p
    for _ in range(V):
        p = b.index(b"\n", p) + 1
    out += b[start:p]
    for _ in range(V):
        put(var())
    sections = [(2, min2), (2, min3)] + ([(3, min3)] if magic == b"QKNG2\n" else [])
    for fields, floor in sections:
        n = u32()
        rows = []
        key = 0
        for _ in range(n):
            key += var()
            vals = [var() for _ in range(fields - 1)]
            if vals[-1] >= floor:
                rows.append((key, vals))
        out += struct.pack("<I", len(rows))
        last = 0
        for k, vals in rows:
            put(k - last)
            for v in vals:
                put(v)
            last = k
        print(f"  {n:,} -> {len(rows):,}", file=sys.stderr)
    open(dst, "wb").write(bytes(out))
    print(f"{src} -> {dst}: {len(b) / 1e6:.1f} MB -> {len(out) / 1e6:.1f} MB", file=sys.stderr)


if __name__ == "__main__":
    main()
