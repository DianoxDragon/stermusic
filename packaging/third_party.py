#!/usr/bin/env python3
"""Liste les bibliothèques Rust compilées dans SterMusic (Linux x86_64, dépendances normales
seulement) avec le texte de leur licence, repris de leurs propres fichiers. Les textes identiques
ne sont écrits qu'une fois. Usage : third_party.py fichier-de-sortie.txt
"""
import glob, hashlib, json, os, subprocess, sys

COMMON = "/usr/share/common-licenses"
NAMES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE", "LICENSE-*", "LICENCE-*")

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--filter-platform", "x86_64-unknown-linux-gnu"]))
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = meta["resolve"]["root"]

# Dépendances normales (pas de compilation ni de tests), en partant du programme.
seen, todo = set(), [root]
while todo:
    nid = todo.pop()
    if nid in seen:
        continue
    seen.add(nid)
    for dep in nodes[nid]["deps"]:
        if any(k["kind"] is None for k in dep["dep_kinds"]):
            todo.append(dep["pkg"])
seen.discard(root)

def license_files(pkg):
    d = os.path.dirname(pkg["manifest_path"])
    files = []
    for pat in NAMES:
        for f in glob.glob(os.path.join(d, pat)) + glob.glob(os.path.join(d, pat.lower())):
            if os.path.isfile(f) and f not in files:
                files.append(f)
    # Polices embarquées (egui) : leurs licences sont rangées à côté d'elles.
    for f in sorted(glob.glob(os.path.join(d, "fonts", "*.txt"))):
        if f not in files:
            files.append(f)
    if pkg.get("license_file"):
        f = os.path.join(d, pkg["license_file"])
        if os.path.isfile(f) and f not in files:
            files.append(f)
    return sorted(files)

def fallback(pkg):
    """Texte de référence quand le paquet n'embarque pas sa licence."""
    lic = pkg.get("license") or ""
    authors = ", ".join(pkg.get("authors") or []) or f"the {pkg['name']} authors"
    if "MIT" in lic:
        return MIT.format(holders=authors)
    if lic == "ISC":
        return ISC.format(holders=authors)
    for name, file in (("Apache-2.0", "Apache-2.0"), ("MPL-2.0", "MPL-2.0"), ("CC0-1.0", "CC0-1.0"), ("BSD", "BSD")):
        if name in lic and os.path.exists(os.path.join(COMMON, file)):
            return open(os.path.join(COMMON, file), encoding="utf-8").read()
    return f"Licence : {lic} (texte non fourni par le paquet ; voir {pkg.get('repository') or 'crates.io'})."

MIT = """Copyright (c) {holders}

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
"""

ISC = """Copyright (c) {holders}

Permission to use, copy, modify, and/or distribute this software for any purpose with or without
fee is hereby granted, provided that the above copyright notice and this permission notice appear
in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE
AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT,
NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE
OF THIS SOFTWARE.
"""

texts = {}   # empreinte -> (texte, [paquets])
rows = []
for pid in sorted(seen, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"])):
    p = pkgs[pid]
    label = f"{p['name']} {p['version']}"
    rows.append(f"{label:<45} {p.get('license') or '?'}")
    bodies = [open(f, encoding="utf-8", errors="replace").read() for f in license_files(p)] or [fallback(p)]
    for body in bodies:
        key = hashlib.sha256(" ".join(body.split()).encode()).hexdigest()
        texts.setdefault(key, [body, []])[1].append(label)

out = [
    "SterMusic — licences des bibliothèques tierces / third-party licenses",
    "=" * 72,
    "",
    "SterMusic est distribué sous GPL-3.0-or-later (fichier LICENSE). Il est compilé avec les",
    "bibliothèques Rust ci-dessous, chacune sous sa propre licence. Les bibliothèques sous MPL-2.0",
    "(Symphonia, resvg…) sont utilisées sans modification ; leur code source est disponible sur",
    "https://crates.io aux versions indiquées.",
    "",
    f"{len(rows)} bibliothèques :",
    "",
    *rows,
    "",
]
for body, users in sorted(texts.values(), key=lambda t: t[1][0]):
    out += ["", "-" * 72, "Utilisé par / used by : " + ", ".join(users), "-" * 72, "", body.rstrip(), ""]
with open(sys.argv[1], "w", encoding="utf-8") as f:
    f.write("\n".join(out) + "\n")
print(f"{len(rows)} bibliothèques, {len(texts)} textes de licence distincts → {sys.argv[1]}")
