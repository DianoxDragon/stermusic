#!/usr/bin/env python3
"""Lists the Rust libraries compiled into SterMusic (Linux x86_64, normal dependencies only)
with the text of their license, taken from their own files. Identical texts are only written
once. Usage: third_party.py output-file.txt
"""
import glob, hashlib, json, os, re, subprocess, sys

# License texts provided by the system: Debian/Ubuntu, then Arch Linux (licenses package).
COMMON = "/usr/share/common-licenses"
SPDX = "/usr/share/licenses/spdx"
NAMES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT", "UNLICENSE", "LICENSE-*", "LICENCE-*")

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--filter-platform", "x86_64-unknown-linux-gnu"]))
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = meta["resolve"]["root"]

# Normal dependencies (no build or test dependencies), starting from the program.
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
    # Embedded fonts (egui): their licenses are stored next to them.
    for f in sorted(glob.glob(os.path.join(d, "fonts", "*.txt"))):
        if f not in files:
            files.append(f)
    if pkg.get("license_file"):
        f = os.path.join(d, pkg["license_file"])
        if os.path.isfile(f) and f not in files:
            files.append(f)
    return sorted(files)

def fallback(pkg):
    """Reference text when the package doesn't ship its license."""
    lic = pkg.get("license") or ""
    authors = ", ".join(pkg.get("authors") or []) or f"the {pkg['name']} authors"
    if "MIT" in lic:
        return MIT.format(holders=authors)
    if lic == "ISC":
        return ISC.format(holders=authors)
    bsd = re.search(r"BSD-[0-9]-Clause", lic)
    for name, file, spdx in (("Apache-2.0", "Apache-2.0", "Apache-2.0"), ("MPL-2.0", "MPL-2.0", "MPL-2.0"),
                             ("CC0-1.0", "CC0-1.0", "CC0-1.0"),
                             ("BSD", "BSD", bsd.group(0) if bsd else "BSD-3-Clause")):
        if name not in lic:
            continue
        for path in (os.path.join(COMMON, file), os.path.join(SPDX, spdx + ".txt")):
            if os.path.exists(path):
                return open(path, encoding="utf-8").read()
    return f"License: {lic} (text not shipped with the package; see {pkg.get('repository') or 'crates.io'})."

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

texts = {}   # fingerprint -> (text, [packages])
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
    "SterMusic — third-party licenses",
    "=" * 72,
    "",
    "SterMusic is released under the GPL-3.0-or-later (see LICENSE). It is compiled with the Rust",
    "libraries below, each under its own license. The MPL-2.0 libraries (Symphonia, resvg…) are",
    "used unmodified; their source code is available on https://crates.io at the listed versions.",
    "",
    f"{len(rows)} libraries:",
    "",
    *rows,
    "",
]
for body, users in sorted(texts.values(), key=lambda t: t[1][0]):
    out += ["", "-" * 72, "Used by: " + ", ".join(users), "-" * 72, "", body.rstrip(), ""]
with open(sys.argv[1], "w", encoding="utf-8") as f:
    f.write("\n".join(out) + "\n")
print(f"{len(rows)} libraries, {len(texts)} distinct license texts → {sys.argv[1]}")
