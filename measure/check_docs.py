"""Contrôle les documents Markdown du dépôt : tables équilibrées, liens et ancres résolus.

    python measure/check_docs.py .

Chaque ligne d'un tableau doit avoir le même nombre de colonnes (barres non échappées) ; chaque
lien vers un fichier du dépôt doit exister, et chaque ancre (`#…`, locale ou d'un autre document)
doit correspondre à un titre.
"""

import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
root = sys.argv[1]
SKIP = {"target", ".git", ".video_agent", "node_modules"}
documents = []
for folder, subfolders, names in os.walk(root):
    subfolders[:] = [name for name in subfolders if name not in SKIP]
    documents += [os.path.join(folder, name) for name in names if name.endswith(".md")]


def anchor(heading):
    heading = re.sub(r"^#+ ", "", heading).lower().replace("’", "")
    return re.sub(r"[^\w\- ]", "", heading, flags=re.UNICODE).replace(" ", "-")


def inside(path):
    return os.path.relpath(path, root).replace(os.sep, "/")


texts = {path: open(path, encoding="utf-8").read() for path in documents}
anchors = {path: {anchor(h) for h in re.findall(r"^(#+ .+)$", text, re.M)} for path, text in texts.items()}
problems = 0
tables = 0
links = 0
for path, text in texts.items():
    for block in re.findall(r"((?:^\|.*\|\n)+)", text, re.M):
        tables += 1
        rows = block.strip().split("\n")
        widths = [len(re.findall(r"(?<!\\)\|", line)) for line in rows]
        if len(set(widths)) != 1:
            problems += 1
            print(f"[table] {inside(path)} : {rows[0][:70]} -> {sorted(set(widths))}")
    for name, fragment in re.findall(r"\]\(([^)#\s]+)(#[^)]+)?\)", text):
        if re.match(r"^[a-z]+:", name) or name.startswith("<"):
            continue
        links += 1
        resolved = os.path.normpath(os.path.join(os.path.dirname(path), name))
        if not os.path.exists(resolved):
            problems += 1
            print(f"[lien] {inside(path)} : fichier absent {name}")
        elif fragment and resolved in anchors and fragment[1:] not in anchors[resolved]:
            problems += 1
            print(f"[ancre] {inside(path)} : {name}{fragment}")
    for link in re.findall(r"\]\(#([^)]+)\)", text):
        links += 1
        if link not in anchors[path]:
            problems += 1
            print(f"[ancre] {inside(path)} : #{link}")
print(f"{len(documents)} documents, {tables} tables, {links} liens vérifiés, {problems} problèmes")
sys.exit(1 if problems else 0)
