"""La chaîne Illumina mesurée : un pool, le séquenceur, le démultiplexage, le décodage.

    python measure/library_pools.py --adn target/release/adn --work /tmp/mesures --pool all

Deux pools de quatre échantillons de 8 Kio d'octets au hasard, index `N701`–`N704` et
`N501`–`N504` :

- `--pool schemes` : un échantillon par schéma (`fountain`, `strands`, `goldman`, `grass`),
  dix-huit scénarios ;
- `--pool rates` : quatre archives `hedges`, un par rendement (¼, ⅓, ½, ¾, dans cet ordre), dix
  scénarios.

Chaque tirage part du pool (`adn library`), passe par le séquenceur (`adn simulate --library`,
lectures de 300 bases sauf mention) et va jusqu'au décodage de chaque échantillon. Les résultats
vont dans `<work>/library-<pool>.jsonl` ; `tables.py library` les met en tableau.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

INDEXES = ["N701+N501", "N702+N502", "N703+N503", "N704+N504"]
SAMPLE_BYTES = 8192
POOLS = {
    "schemes": [
        ("echantillon1", "fountain", ()),
        ("echantillon2", "strands", ()),
        ("echantillon3", "goldman", ()),
        ("echantillon4", "grass", ()),
    ],
    "rates": [
        ("quart", "hedges", ("--code-rate", "1/4")),
        ("tiers", "hedges", ("--code-rate", "1/3")),
        ("demi", "hedges", ("--code-rate", "1/2")),
        ("troisquarts", "hedges", ("--code-rate", "3/4")),
    ],
}


def at(coverage):
    return ["--coverage", str(coverage)]


SYNTHESIS_NAME = "synthèse : indels 3 %, couverture 10"
SYNTHESIS = [
    "--synthesis-substitution-rate", "0.01",
    "--synthesis-insertion-rate", "0.01",
    "--synthesis-deletion-rate", "0.01",
] + at(10)


def indels(each, coverage):
    name = (
        f"indels {common.percent(3 * each)} ({common.percent(each)} de chaque sorte), "
        f"couverture {coverage}"
    )
    return name, common.indels(each) + at(coverage)


SCENARIOS = {
    # Ce que la bibliothèque elle-même doit traverser : les lectures d'index, les lectures
    # étrangères, la longueur des lectures. Le reste est celui du canal.
    "schemes": [
        ("référence", at(10)),
        ("sauts d'index 0,1 %", at(10) + ["--index-hopping", "0.001"]),
        ("sauts d'index 1 %", at(10) + ["--index-hopping", "0.01"]),
        ("sauts d'index 5 %", at(10) + ["--index-hopping", "0.05"]),
        ("sauts d'index 10 %", at(10) + ["--index-hopping", "0.10"]),
        ("sauts d'index 30 %", at(10) + ["--index-hopping", "0.30"]),
        ("substitutions 0,5 %", ["--substitution-rate", "0.005"] + at(10)),
        ("substitutions 1 %", ["--substitution-rate", "0.01"] + at(10)),
        ("substitutions 2 %", ["--substitution-rate", "0.02"] + at(10)),
        ("index 1 %", at(10) + ["--index-substitution-rate", "0.01"]),
        ("index 5 %", at(10) + ["--index-substitution-rate", "0.05"]),
        ("substitutions 1 % et sauts 5 %",
         ["--substitution-rate", "0.01", "--index-hopping", "0.05"] + at(10)),
        ("synthèse 1 %", ["--synthesis-substitution-rate", "0.01"] + at(10)),
        ("perte 10 %", ["--dropout", "0.10"] + at(10)),
        ("couverture 3", at(3)),
        ("couverture 1", at(1)),
        ("lectures de 200 bases", at(10) + ["--read-length", "200"]),
        ("lectures de 150 bases", at(10) + ["--read-length", "150"]),
    ],
    # Ce que le schéma `hedges` traverse une fois la bibliothèque passée.
    "rates": [
        ("référence, couverture 10", at(10)),
        ("indels 1 % (0,33 % de chaque sorte), couverture 10", common.indels(0.0033) + at(10)),
        indels(0.01, 10),
        ("indels 3 %, couverture 20", common.indels(0.01) + at(20)),
        ("substitutions 1 %, couverture 10", ["--substitution-rate", "0.01"] + at(10)),
        (SYNTHESIS_NAME, SYNTHESIS),
        ("sauts d'index 10 %, couverture 10", at(10) + ["--index-hopping", "0.10"]),
        ("lectures de 200 bases, couverture 10", at(10) + ["--read-length", "200"]),
        ("lectures de 150 bases, couverture 10", at(10) + ["--read-length", "150"]),
        ("couverture 3", at(3)),
    ],
}


def build_pool(adn, work, pool):
    """Encode les échantillons et assemble le pool ; rend son chemin et son rapport."""
    samples = []
    for rank, (name, scheme, options) in enumerate(POOLS[pool]):
        source = common.write_work(work, f"{name}.bin", common.random_bytes(SAMPLE_BYTES, 100 + rank))
        archive = common.encode(
            adn, source, os.path.join(work, f"{name}.adn.fasta"), scheme, *options
        )
        samples.append(f"{name}={archive}={INDEXES[rank]}")
    arguments = ["library", "-o", os.path.join(work, "pool.fasta")]
    for sample in samples:
        arguments += ["--sample", sample]
    return arguments[2], common.report(adn, *arguments)


def main():
    common.prepare_output()
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    parser = argparse.ArgumentParser(description="Chaîne Illumina mesurée sur deux pools.")
    parser.add_argument("--adn", default=common.default_binary(root), help="le binaire `adn`")
    parser.add_argument("--work", default=os.path.join(os.path.expanduser("~"), ".adn-mesures"))
    parser.add_argument("--pool", choices=[*POOLS, "all"], default="all")
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()

    for pool in POOLS if args.pool == "all" else [args.pool]:
        path, built = build_pool(args.adn, os.path.join(args.work, f"library-{pool}"), pool)
        print(f"pool : {len(POOLS[pool])} échantillons, {built['molecules']} molécules", flush=True)
        out = os.path.join(args.work, f"library-{pool}.jsonl")
        with open(out, "w", encoding="utf-8") as results:
            for name, extra in SCENARIOS[pool]:
                command = [
                    "simulate", "-i", path, "--library", "-n", str(args.trials),
                    "--seed", str(args.seed), *extra,
                ]
                seconds, code, out_text, err = common.timed(args.adn, "--json", *command)
                if code != 0:
                    raise SystemExit(
                        f"adn {' '.join(command)} a échoué : {err.strip() or out_text.strip()}"
                    )
                parsed = json.loads(out_text)
                record = {
                    "scenario": name,
                    "options": extra,
                    "trials": args.trials,
                    "recuperes": parsed["bilan"]["recuperes"],
                    "duree_s": round(seconds, 2),
                    "fausses": 0,
                    "inconnues": 0,
                    "ambigues": 0,
                    "sautees": 0,
                    "paires": 0,
                    "samples": {},
                }
                for trial in parsed["tirages"]:
                    record["fausses"] += sum(sample["fausses"] for sample in trial["echantillons"])
                    record["inconnues"] += trial["inconnues"]
                    record["ambigues"] += trial["ambigues"]
                    record["sautees"] += trial["sautees"]
                    record["paires"] += trial["paires"]
                    for sample in trial["echantillons"]:
                        counts = record["samples"].setdefault(
                            sample["nom"], {"exact": 0, "echec": 0}
                        )
                        counts["exact" if sample["recuperation"] == "Exact" else "echec"] += 1
                results.write(json.dumps(record, ensure_ascii=False) + "\n")
                per_sample = "  ".join(
                    f"{sample} {counts['exact']}/{args.trials}"
                    for sample, counts in record["samples"].items()
                )
                print(
                    f"{name:<52} {record['recuperes']}/{args.trials}  fausses {record['fausses']}  "
                    f"inconnues {record['inconnues']}  {record['duree_s']} s   {per_sample}",
                    flush=True,
                )
    print("fin")


if __name__ == "__main__":
    main()
