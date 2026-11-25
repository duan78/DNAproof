"""Durées de la ligne de commande, la machine au repos.

    python measure/timings.py --adn target/release/adn --work /tmp/mesures

Deux parties, sur des octets au hasard :

- **Décodage de `hedges`** : pour chaque rendement, sur l'archive de 64 Kio du dossier de travail,
  l'archive intacte (décodée six fois, la première échauffe), puis dix lectures par brin, intactes
  et au taux nominal d'erreurs des trois sortes (1 % à ¾, 3 % à ½, 6 % à ⅓, 10 % à ¼) ;
- **Chaîne Illumina** : un pool de quatre archives de 64 Kio (un par schéma), assemblé, séquencé
  une fois en couverture 10, démultiplexé par le pool puis par des index déclarés sans lecture i5 ;
  puis un échantillon seul d'un mébioctet.

Rien d'autre ne doit tourner : les durées dépendent de la machine.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

RATES = [("3/4", 0.0033), ("1/2", 0.01), ("1/3", 0.02), ("1/4", 0.033)]
SCHEMES = ["fountain", "strands", "goldman", "grass"]
INDEXES = ["N701+N501", "N702+N502", "N703+N503", "N704+N504"]


def seconds(seconds_value):
    return f"{seconds_value:.2f} s"


def hedges(adn, work):
    source = common.write_work(work, "alea-64kio.bin", common.random_bytes(64 * 1024, 20241124))
    output = os.path.join(work, "timing", "decoded.bin")
    os.makedirs(os.path.dirname(output), exist_ok=True)
    for rate, each in RATES:
        archive = common.encode(
            adn, source, os.path.join(work, f"alea-64kio-hedges-{rate.replace('/', 'sur')}.fasta"),
            "hedges", "--code-rate", rate,
        )
        # La première mesure échauffe ; on garde les cinq suivantes.
        intact = [common.timed(adn, "decode", "-i", archive, "-o", output)[0] for _ in range(6)][1:]
        rows = {}
        for label, extra in (("intactes", ["--substitution-rate", "0"]), ("nominal", common.indels(each))):
            arguments = [
                "simulate", "--archive", "-i", archive, "--coverage", "10", "-n", "5", "--seed", "7", *extra
            ]
            seconds_spent, code, out, err = common.timed(adn, "--json", *arguments)
            if code != 0:
                raise SystemExit(f"adn {' '.join(arguments)} a échoué : {err.strip() or out.strip()}")
            parsed = json.loads(out)
            rows[label] = (seconds_spent / 5, parsed["tirages"][0]["lectures"], parsed["bilan"]["recuperes"])
        print(
            f"hedges {rate} : archive intacte {sum(intact) / len(intact) * 1000:.0f} ms ; "
            f"couverture 10 intactes {seconds(rows['intactes'][0])} pour {rows['intactes'][1]} lectures "
            f"({rows['intactes'][1] / rows['intactes'][0]:.0f} lectures/s) ; "
            f"indels {3 * each:.1%} {seconds(rows['nominal'][0])} "
            f"({rows['nominal'][1] / rows['nominal'][0]:.0f} lectures/s), "
            f"{rows['nominal'][2]}/5 restitués",
            flush=True,
        )


def checked(adn, *arguments):
    """Lance une commande et rend sa durée ; s'arrête si elle échoue."""
    seconds_spent, code, out, err = common.timed(adn, *arguments)
    if code != 0:
        raise SystemExit(f"adn {' '.join(arguments)} a échoué : {err.strip() or out.strip()}")
    return seconds_spent, out


def illumina(adn, work):
    work = os.path.join(work, "timings-illumina")
    samples = []
    for rank, scheme in enumerate(SCHEMES):
        name = f"echantillon{rank + 1}"
        source = common.write_work(work, f"{name}.bin", common.random_bytes(64 * 1024, 300 + rank))
        archive = common.encode(adn, source, os.path.join(work, f"{name}.adn.fasta"), scheme)
        samples.append(f"{name}={archive}={INDEXES[rank]}")
    pool = os.path.join(work, "pool.fasta")
    arguments = ["library", "-o", pool]
    for sample in samples:
        arguments += ["--sample", sample]
    spent, out = checked(adn, "--json", *arguments)
    molecules = json.loads(out)["molecules"]
    print(f"adn library, 4 archives de 64 Kio, un par schéma : {spent * 1000:.0f} ms, {molecules} molécules", flush=True)

    reads = os.path.join(work, "lectures")
    spent, out = checked(adn, "--json", "simulate", "-i", pool, "--library", "--coverage", "10", "-n", "1")
    pairs = json.loads(out)["tirages"][0]["paires"]
    print(
        f"adn simulate --library, un tirage (séquenceur, démultiplexage, décodage) : {seconds(spent)}, {pairs} paires",
        flush=True,
    )
    # Un second tirage, pareil mais gardé en FASTQ pour le démultiplexage : il ne se chronomètre pas.
    checked(adn, "--json", "simulate", "-i", pool, "--library", "--coverage", "10", "-n", "1", "--reads", reads)

    spent, _ = checked(
        adn, "demux", "--r1", f"{reads}/R1.fastq", "--r2", f"{reads}/R2.fastq",
        "--i7", f"{reads}/I1.fastq", "--i5", f"{reads}/I2.fastq", "--library", pool,
        "--output", os.path.join(work, "par-pool"),
    )
    print(f"adn demux, échantillons pris du pool : {seconds(spent)}, soit {pairs / spent / 1000:.0f} milliers de paires par seconde", flush=True)

    declared = ["demux", "--r1", f"{reads}/R1.fastq", "--r2", f"{reads}/R2.fastq", "--i7", f"{reads}/I1.fastq"]
    for rank in range(len(SCHEMES)):
        declared += ["--sample", f"{INDEXES[rank]}=echantillon{rank + 1}"]
    spent, _ = checked(adn, *declared, "--output", os.path.join(work, "par-index"))
    print(f"adn demux, index déclarés, sans lecture d'index i5 : {seconds(spent)}", flush=True)

    source = common.write_work(work, "seul.bin", common.random_bytes(1 << 20, 400))
    archive = common.encode(adn, source, os.path.join(work, "seul.adn.fasta"), "fountain")
    pool = os.path.join(work, "seul-pool.fasta")
    checked(adn, "library", "-o", pool, "--sample", f"seul={archive}=N701+N501")
    reads = os.path.join(work, "lectures-seul")
    spent, out = checked(adn, "--json", "simulate", "-i", pool, "--library", "--coverage", "10", "-n", "1")
    pairs = json.loads(out)["tirages"][0]["paires"]
    print(f"adn simulate --library, un échantillon d'un mébioctet : {seconds(spent)}, {pairs} paires", flush=True)
    checked(adn, "--json", "simulate", "-i", pool, "--library", "--coverage", "10", "-n", "1", "--reads", reads)
    spent, _ = checked(
        adn, "demux", "--r1", f"{reads}/R1.fastq", "--r2", f"{reads}/R2.fastq",
        "--i7", f"{reads}/I1.fastq", "--i5", f"{reads}/I2.fastq", "--library", pool,
        "--output", os.path.join(work, "seul-demux"),
    )
    print(f"adn demux, un échantillon d'un mébioctet : {seconds(spent)}, soit {pairs / spent / 1000:.0f} milliers de paires par seconde", flush=True)


def main():
    common.prepare_output()
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    parser = argparse.ArgumentParser(description="Durées de la ligne de commande.")
    parser.add_argument("--adn", default=common.default_binary(root), help="le binaire `adn`")
    parser.add_argument("--work", default=os.path.join(os.path.expanduser("~"), ".adn-mesures"))
    parser.add_argument("--part", choices=["hedges", "illumina", "all"], default="all")
    args = parser.parse_args()
    if args.part in ("hedges", "all"):
        hedges(args.adn, args.work)
    if args.part in ("illumina", "all"):
        illumina(args.adn, args.work)
    print("fin")


if __name__ == "__main__":
    main()
