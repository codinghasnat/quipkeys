#!/usr/bin/env python3
"""Turn extracted NLTK corpora into chatty.txt (fiction, web/chat, reviews) and news.txt.
Usage: python3 -I scripts/prep_nltk_text.py <dir-with-extracted-corpora>"""
import glob, html, os, re, sys
d = sys.argv[1]
def paras(path):
    txt = open(path, encoding="latin-1").read()
    for p in re.split(r"\n\s*\n", txt):
        p = " ".join(p.split())
        if p:
            yield p
with open(os.path.join(d, "chatty.txt"), "w") as out:
    for f in glob.glob(f"{d}/gutenberg/*.txt") + glob.glob(f"{d}/movie_reviews/*/*/*.txt"):
        for p in paras(f):
            out.write(p + "\n")
    for f in glob.glob(f"{d}/webtext/*.txt"):
        for line in open(f, encoding="latin-1"):
            out.write(line.strip() + "\n")
    for f in glob.glob(f"{d}/nps_chat/*.xml"):
        for m in re.finditer(r"<Post[^>]*>(.*?)</Post>", open(f, encoding="latin-1").read(), re.S):
            out.write(html.unescape(m.group(1)).strip() + "\n")
with open(os.path.join(d, "news.txt"), "w") as out:
    for f in glob.glob(f"{d}/abc/*.txt") + glob.glob(f"{d}/reuters/training/*") + glob.glob(f"{d}/reuters/test/*"):
        for p in paras(f):
            out.write(p + "\n")
