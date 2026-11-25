"""Outils communs aux mesures : lancer ``adn``, fabriquer des fichiers au hasard, tenir un journal.

Tous les scripts de ce dossier prennent le binaire ``adn`` en argument et écrivent leurs
résultats en JSON Lines (un objet par ligne), que ``tables.py`` met en tableau Markdown.
"""

import json
import os
import subprocess
import sys
import time


def prepare_output():
    """Affiche du français dans une console Windows, quel que soit son encodage."""
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")


def default_binary(root):
    """Le binaire ``adn`` d'une construction ``--release`` du dépôt, si elle existe."""
    name = "adn.exe" if os.name == "nt" else "adn"
    return os.path.join(root, "target", "release", name)


def run(binary, *args):
    """Lance ``adn`` et rend (code de sortie, sortie standard, sortie d'erreur)."""
    done = subprocess.run(
        [binary, *args], capture_output=True, text=True, encoding="utf-8"
    )
    return done.returncode, done.stdout, done.stderr


def timed(binary, *args):
    """Lance ``adn`` et rend (durée en secondes, code de sortie, sortie standard, sortie d'erreur)."""
    started = time.perf_counter()
    code, out, err = run(binary, *args)
    return time.perf_counter() - started, code, out, err


def report(binary, *args):
    """Le rapport JSON d'une commande ``adn --json`` ; s'arrête si elle échoue."""
    code, out, err = run(binary, "--json", *args)
    if code != 0:
        raise SystemExit(f"adn {' '.join(args)} a échoué : {err.strip() or out.strip()}")
    return json.loads(out)


def random_bytes(size, seed):
    """Des octets au hasard, reproductibles : un xorshift 64 bits."""
    state = seed | 1
    out = bytearray(size)
    for index in range(size):
        state ^= (state << 13) & 0xFFFFFFFFFFFFFFFF
        state ^= state >> 7
        state ^= (state << 17) & 0xFFFFFFFFFFFFFFFF
        out[index] = state & 0xFF
    return bytes(out)


def write_work(work, name, content):
    """Écrit un fichier de travail et rend son chemin."""
    os.makedirs(work, exist_ok=True)
    path = os.path.join(work, name)
    with open(path, "wb") as out:
        out.write(content)
    return path


def encode(binary, source, path, scheme, *extra):
    """Encode un fichier avec un schéma ; l'archive n'est pas recréée si elle existe déjà."""
    if not os.path.exists(path):
        code, out, err = run(
            binary, "encode", "-i", source, "-o", path, "-a", scheme, "-c", "none", *extra
        )
        if code != 0:
            raise SystemExit(f"encodage {scheme} en {path} : {err.strip() or out.strip()}")
    return path


def percent(value):
    """Un taux en pourcent, à la française : « 1,5 % »."""
    return f"{100 * value:g} %".replace(".", ",")


def indels(each):
    """Autant de substitutions, d'insertions et de suppressions que cela, par nucléotide."""
    return ["--substitution-rate", str(each), "--insertion-rate", str(each), "--deletion-rate", str(each)]


def bursts(mean, length):
    """Des erreurs en salves de ``length`` nucléotides où une base sur deux est fausse."""
    rate = mean / (length * (1.0 - mean))
    return ["--burst-rate", f"{rate:.6g}", "--burst-length", str(length), "--burst-error", "0.5"]


class Journal:
    """Un fichier de résultats en JSON Lines, et une ligne à l'écran par mesure."""

    def __init__(self, path):
        self.path = path
        os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
        # Le fichier est recréé à chaque mesure complète.
        with open(self.path, "w", encoding="utf-8") as out:
            out.write("")

    def add(self, record, shown):
        with open(self.path, "a", encoding="utf-8") as out:
            out.write(json.dumps(record, ensure_ascii=False) + "\n")
        print(shown, flush=True)
