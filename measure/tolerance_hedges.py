"""Matrice de tolérance du schéma `hedges` : insertions et suppressions, par rendement.

    python measure/tolerance_hedges.py --adn target/release/adn --work /tmp/mesures

Conditions : un fichier de 64 Kio d'octets au hasard, `--trials` tirages indépendants par cas
(dix par défaut), à partir de la graine `--seed`. Les taux sont ceux des trois sortes d'erreurs à
la fois, en parts égales : « indels 3 % » veut dire 1 % de substitutions, 1 % d'insertions et
1 % de suppressions par nucléotide.

- `--group matrix` (défaut) : les cinq autres schémas devant 1,5 % d'indels, puis les quatre
  rendements de `hedges` (trente-trois cas).
- `--group extra` : les six schémas devant 3 % d'erreurs de synthèse des trois sortes, et les
  limites de chaque rendement (quatorze cas).

Les résultats vont dans `<work>/tolerance-hedges.jsonl` ; `tables.py tolerance` les met en
tableau. Avec `--resume`, une mesure reprise saute les cas déjà mesurés.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

LDPC = ("--rs-parity", "20")
QUARTER = ("--code-rate", "1/4")
THIRD = ("--code-rate", "1/3")
HALF = ("--code-rate", "1/2")
THREE_QUARTERS = ("--code-rate", "3/4")


def at(coverage):
    return ["--coverage", str(coverage)]


def indels(each, coverage, total=None):
    """Un cas « indels » : son intitulé et ses options."""
    share = 3 * each if total is None else total
    name = (
        f"indels {common.percent(share)} ({common.percent(each)} de chaque sorte), "
        f"couverture {coverage}"
    )
    return name, common.indels(each) + at(coverage)


def loss(percent, coverage):
    return f"perte {common.percent(percent / 100)}, couverture {coverage}", [
        "--dropout", str(percent / 100)
    ] + at(coverage)


SYNTHESIS_NAME = "synthèse : indels 3 %, couverture 10"
SYNTHESIS = [
    "--synthesis-substitution-rate", "0.01",
    "--synthesis-insertion-rate", "0.01",
    "--synthesis-deletion-rate", "0.01",
] + at(10)

OTHERS = ("fountain", ()), ("strands", ()), ("goldman", ()), ("grass", ()), ("ldpc", LDPC)

MATRIX = [
    # Les cinq autres schémas, devant des insertions et des suppressions.
    *[(name, name, options, *indels(0.005, 10)) for name, options in OTHERS],
    # Rendement ½.
    ("hedges-1/2", "hedges", HALF, *indels(0.005, 5)),
    ("hedges-1/2", "hedges", HALF, *indels(0.01, 5)),
    ("hedges-1/2", "hedges", HALF, *indels(0.01, 10)),
    ("hedges-1/2", "hedges", HALF, "substitutions 3 %, couverture 10",
     ["--substitution-rate", "0.03"] + at(10)),
    ("hedges-1/2", "hedges", HALF, SYNTHESIS_NAME, SYNTHESIS),
    ("hedges-1/2", "hedges", HALF, *loss(7, 10)),
    ("hedges-1/2", "hedges", HALF, *loss(8, 10)),
    ("hedges-1/2", "hedges", HALF, *loss(10, 10)),
    ("hedges-1/2", "hedges", HALF, *indels(0.015, 10)),
    ("hedges-1/2", "hedges", HALF, *indels(0.01, 20)),
    ("hedges-1/2", "hedges", HALF, "substitutions 6 %, couverture 20",
     ["--substitution-rate", "0.06"] + at(20)),
    ("hedges-1/2", "hedges", HALF, *indels(0.02, 20)),
    ("hedges-1/2", "hedges", HALF, "substitutions 5 % en salves de 4, couverture 10",
     common.bursts(0.05, 4) + at(10)),
    ("hedges-1/2", "hedges", HALF, "substitutions 10 % en salves de 4, couverture 20",
     common.bursts(0.10, 4) + at(20)),
    # Rendement ¾.
    ("hedges-3/4", "hedges", THREE_QUARTERS, "substitutions 1 %, couverture 10",
     ["--substitution-rate", "0.01"] + at(10)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, *indels(0.00333, 10, total=0.01)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, *indels(0.005, 10)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, *loss(8, 10)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, *indels(0.005, 20)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, *indels(0.01, 20)),
    # Rendement ⅓.
    ("hedges-1/3", "hedges", THIRD, "substitutions 3 %, couverture 10",
     ["--substitution-rate", "0.03"] + at(10)),
    ("hedges-1/3", "hedges", THIRD, *indels(0.01, 5)),
    ("hedges-1/3", "hedges", THIRD, *loss(8, 10)),
    ("hedges-1/3", "hedges", THIRD, *indels(0.02, 10)),
    ("hedges-1/3", "hedges", THIRD, *indels(0.02, 20)),
    ("hedges-1/3", "hedges", THIRD, *indels(0.033, 20)),
    # Rendement ¼.
    ("hedges-1/4", "hedges", QUARTER, "substitutions 5 %, couverture 10",
     ["--substitution-rate", "0.05"] + at(10)),
    ("hedges-1/4", "hedges", QUARTER, *loss(7, 10)),
    ("hedges-1/4", "hedges", QUARTER, *indels(0.02, 5)),
    ("hedges-1/4", "hedges", QUARTER, *indels(0.033, 10)),
    ("hedges-1/4", "hedges", QUARTER, *indels(0.033, 20)),
    ("hedges-1/4", "hedges", QUARTER, *indels(0.04, 10)),
]

EXTRA = [
    *[(name, name, options, SYNTHESIS_NAME, SYNTHESIS) for name, options in OTHERS],
    ("hedges-1/4", "hedges", QUARTER, *indels(0.05, 10)),
    ("hedges-1/4", "hedges", QUARTER, *indels(0.05, 20)),
    ("hedges-1/4", "hedges", QUARTER, *indels(0.06, 20)),
    ("hedges-1/3", "hedges", THIRD, *indels(0.04, 20)),
    ("hedges-1/2", "hedges", HALF, *indels(0.02, 10)),
    ("hedges-1/2", "hedges", HALF, *indels(0.03, 20)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, *indels(0.005, 5)),
    ("hedges-3/4", "hedges", THREE_QUARTERS, "indels 2 % (0,67 % de chaque sorte), couverture 20",
     common.indels(0.0067) + at(20)),
    ("hedges-1/4", "hedges", QUARTER, *loss(8, 10)),
]


def main():
    common.prepare_output()
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    parser = argparse.ArgumentParser(description="Matrice de tolérance du schéma hedges.")
    parser.add_argument("--adn", default=common.default_binary(root), help="le binaire `adn`")
    parser.add_argument(
        "--work",
        default=os.path.join(os.path.expanduser("~"), ".adn-mesures"),
        help="où écrire les fichiers de travail",
    )
    parser.add_argument("--out", help="le fichier de résultats (par défaut : <work>/tolerance-hedges.jsonl)")
    parser.add_argument("--trials", type=int, default=10)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--group", choices=["matrix", "extra", "all"], default="matrix")
    parser.add_argument("--resume", action="store_true", help="passer les cas déjà mesurés")
    args = parser.parse_args()
    out = args.out or os.path.join(args.work, "tolerance-hedges.jsonl")

    source = common.write_work(args.work, "alea-64kio.bin", common.random_bytes(64 * 1024, 20241124))

    done = set()
    if args.resume and os.path.exists(out):
        with open(out, encoding="utf-8") as saved:
            for line in saved:
                record = json.loads(line)
                done.add((record["schema"], record["scenario"]))
    elif os.path.exists(out):
        os.remove(out)

    def archive(scheme, options):
        """L'archive du schéma, créée s'il le faut."""
        suffix = "".join(part.strip("-").replace("/", "sur") for part in options)
        name = f"alea-64kio-{scheme}{('-' + suffix) if suffix else ''}.fasta"
        return common.encode(args.adn, source, os.path.join(args.work, name), scheme, *options)

    cases = MATRIX if args.group == "matrix" else EXTRA if args.group == "extra" else MATRIX + EXTRA
    for label, scheme, options, name, channel in cases:
        if (label, name) in done:
            print(f"déjà mesuré : {label} | {name}", flush=True)
            continue
        path = archive(scheme, options)
        command = [
            "simulate", "--archive", "-i", path,
            "-n", str(args.trials), "--seed", str(args.seed), *channel,
        ]
        seconds, code, out_text, err = common.timed(args.adn, "--json", *command)
        if code != 0:
            raise SystemExit(f"adn {' '.join(command)} a échoué : {err.strip() or out_text.strip()}")
        parsed = json.loads(out_text)
        trials = parsed["tirages"]
        record = {
            "schema": label,
            "scenario": name,
            "recuperes": parsed["bilan"]["recuperes"],
            "echecs": parsed["bilan"]["echecs"],
            "faux": parsed["bilan"]["faux"],
            "tirages": parsed["bilan"]["tirages"],
            "oligos": trials[0]["oligos"],
            "lectures": trials[0]["lectures"],
            "perdus_moyen_pct": round(
                100 * sum(trial["oligos_perdus"] for trial in trials) / len(trials) / trials[0]["oligos"], 1
            ),
            "duree_s": round(seconds, 2),
        }
        with open(out, "a", encoding="utf-8") as results:
            results.write(json.dumps(record, ensure_ascii=False) + "\n")
        print(f"{record['recuperes']}/{record['tirages']} {label} | {name} | {record['duree_s']} s", flush=True)
    print("fin")


if __name__ == "__main__":
    main()
