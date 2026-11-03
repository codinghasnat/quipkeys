#!/usr/bin/env python3
"""Extract plain paragraphs from TFDS Wiki40B tfrecord shards (no TensorFlow needed).

Usage: python3 -I scripts/extract_wiki40b.py shard1.tfrecord [...] > wiki.txt
Shards: https://storage.googleapis.com/tfds-data/datasets/wiki40b/Wiki40B.en/1.0.0/
One paragraph per output line. Text is CC BY-SA (Wikipedia).
"""
import re, struct, sys

CUT = re.compile(r"[\x00-\x09\x0b-\x1f]")
out = sys.stdout
for path in sys.argv[1:]:
    with open(path, "rb") as f:
        while True:
            head = f.read(12)
            if len(head) < 12:
                break
            (n,) = struct.unpack("<Q", head[:8])
            data = f.read(n)
            f.read(4)
            s = data.decode("utf-8", "ignore")
            for chunk in s.split("_START_PARAGRAPH_\n")[1:]:
                para = CUT.split(chunk.split("\n", 1)[0], 1)[0]
                for line in para.split("_NEWLINE_"):
                    line = line.strip()
                    if line:
                        out.write(line + "\n")
