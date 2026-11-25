"""Les résultats des mesures en tableaux Markdown, prêts pour `docs/`.

    python measure/tables.py tolerance <work>/tolerance-hedges.jsonl
    python measure/tables.py library <work>/library-rates.jsonl
    python measure/tables.py bench <journal criterion>
"""

import json
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")

RATES = ["hedges-3/4", "hedges-1/2", "hedges-1/3", "hedges-1/4"]
SCALE = {"ns": 1e-9, "µs": 1e-6, "ms": 1e-3, "s": 1.0}


def read(*paths):
    """Les enregistrements d'un ou de plusieurs fichiers de résultats."""
    records = []
    for path in paths:
        with open(path, encoding="utf-8") as source:
            records += [json.loads(line) for line in source]
    return records


def number(text):
    """Un taux écrit à la française (« 1,5 »)."""
    return float(text.replace(",", "."))


def thousands(value):
    return f"{int(value):,}".replace(",", "\u202f")


def order(label):
    """L'ordre des lignes de la matrice : les indels, les substitutions, les salves, la synthèse,
    la perte."""
    coverage = int(label.split("couverture ")[1])
    if label.startswith("indels "):
        return (0, number(label[len("indels "):].split(" %")[0]), coverage)
    if label.startswith("substitutions ") and "salves" in label:
        return (3, number(label.split(" %")[0].split("substitutions ")[1]), coverage)
    if label.startswith("substitutions "):
        return (2, number(label.split(" %")[0].split("substitutions ")[1]), coverage)
    if label.startswith("synthèse"):
        return (4, 0.0, coverage)
    return (5, number(label.split("perte ")[1].split(" %")[0]), coverage)


def tolerance(paths):
    """La matrice du schéma `hedges`, puis le bilan des autres schémas."""
    cells, others = {}, {}
    for record in read(*paths):
        assert record["faux"] == 0, record
        if record["schema"] not in RATES:
            others[(record["schema"], record["scenario"])] = record
            continue
        # « indels 3 % (1 % de chaque sorte), couverture 10 » : le taux des trois sortes suffit.
        label = re.sub(r" \([^)]*de chaque sorte\)", "", record["scenario"])
        key = (record["schema"], label)
        assert key not in cells, key
        cells[key] = record
    print("| Scénario | `hedges` ¾ | `hedges` ½ | `hedges` ⅓ | `hedges` ¼ |")
    print("|----------|:----------:|:----------:|:----------:|:----------:|")
    for label in sorted({label for _, label in cells}, key=order):
        row = []
        for rate in RATES:
            record = cells.get((rate, label))
            row.append("—" if record is None else f"{record['recuperes']}/{record['tirages']}")
        print(f"| {label} | " + " | ".join(row) + " |")
    print()
    print(
        f"{len(cells)} cas, {sum(r['tirages'] for r in cells.values())} tirages ; "
        f"autres schémas : {len(others)} cas, {sum(r['tirages'] for r in others.values())} tirages, "
        f"{sum(r['recuperes'] for r in others.values())} restitués."
    )
    for (schema, scenario), record in sorted(others.items()):
        print(f"  {schema} | {scenario} | {record['recuperes']}/{record['tirages']}")


def library(paths):
    """Un pool Illumina : une ligne par scénario."""
    print("| Scénario | Tirages entièrement restitués | Attributions fausses | Paires sans réponse | Échantillons restitués |")
    print("|---|:---:|:---:|:---:|---|")
    for record in read(*paths):
        per_sample = ", ".join(
            f"{counts['exact']}/{record['trials']}" for counts in record["samples"].values()
        )
        shown = record["fausses"] or 0
        print(
            f"| {record['scenario']} | {record['recuperes']} sur {record['trials']} | "
            f"{thousands(shown) if shown else 0} | {thousands(record['inconnues'])} | {per_sample} |"
        )


def french(seconds):
    """Une durée en notation française, à la précision des documents : « 14 µs », « 2,31 s »."""
    if seconds < 1e-6:
        return f"{seconds * 1e9:.0f} ns"
    if seconds < 1e-3:
        milli = seconds * 1e6
        digits = 0 if milli >= 100 else (1 if milli >= 10 else 2)
        return f"{milli:.{digits}f} µs".replace(".", ",")
    if seconds < 1:
        milli = seconds * 1e3
        digits = 0 if milli >= 100 else 1
        return f"{milli:.{digits}f} ms".replace(".", ",")
    return f"{seconds:.2f} s".replace(".", ",")


def bench(paths):
    """Les médiannes d'un journal criterion."""
    for path in paths:
        lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
        for index, line in enumerate(lines):
            # Les blocs « change: » comparent à la mesure précédente : seuls comptent les temps
            # absolus, dont la ligne ne porte pas de pourcentage.
            if "time:" not in line or "Benchmarking" in line or "%" in line:
                continue
            head, tail = line.split("time:")
            name = head.strip() or lines[index - 1].strip()
            low, mid, high = re.findall(r"[\d.]+ (?:ns|µs|ms|s)", tail)
            values = []
            for token in (low, mid, high):
                amount, unit = token.split()
                values.append(float(amount) * SCALE[unit])
            print(f"{name:<52} {french(values[1]):>9}   [{french(values[0])} ; {french(values[2])}]")


if __name__ == "__main__":
    kind, *files = sys.argv[1:]
    if kind == "tolerance":
        tolerance(files)
    elif kind == "library":
        library(files)
    elif kind == "bench":
        bench(files)
    else:
        raise SystemExit(__doc__)
