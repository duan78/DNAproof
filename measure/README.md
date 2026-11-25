# Mesurer

Les chiffres de `docs/` viennent d'ici : des scripts Python (la bibliothèque standard suffit)
qui lancent la ligne de commande `adn` et écrivent leurs résultats en JSON Lines.

```sh
cargo build --release --locked -p adn-cli
python measure/tolerance_hedges.py --adn target/release/adn --work /tmp/mesures --group all
python measure/library_pools.py --adn target/release/adn --work /tmp/mesures --pool all
python measure/timings.py --adn target/release/adn --work /tmp/mesures
python measure/tables.py tolerance /tmp/mesures/tolerance-hedges.jsonl
python measure/tables.py library /tmp/mesures/library-rates.jsonl
```

| Script | Ce qu'il mesure | Où vont ses chiffres |
|---|---|---|
| `tolerance_hedges.py` | le schéma `hedges` devant les insertions et suppressions, par rendement ; les autres schémas à 1,5 % d'indels et devant 3 % d'erreurs de synthèse | [Schémas d'encodage](../docs/encoding_schemes.md#insertions-et-suppressions-hedges) |
| `library_pools.py` | la chaîne Illumina sur deux pools : un échantillon par schéma, puis un par rendement de `hedges` | [Bibliothèques Illumina](../docs/LIBRARY.md#mesures) |
| `timings.py` | les durées de décodage d'une archive `hedges` | [Performances](../docs/PERFORMANCE.md#petites-tailles) |
| `cargo bench -p adn-core` | les codecs, par schéma et par taille, et les briques prises isolément | [Performances](../docs/PERFORMANCE.md) |
| `tables.py` | met les résultats en tableaux Markdown | — |
| `check_docs.py` | contrôle les tables, les liens et les ancres de tous les documents | — |

`--work` reçoit les archives et les résultats (par défaut `~/.adn-mesures`) ; il suffit de le
donner à `tables.py`. Les matrices des autres schémas se refont avec `adn simulate` directement,
comme indiqué dans [Refaire ces mesures](../docs/encoding_schemes.md#refaire-ces-mesures).

Les durées veulent une machine au repos. Les matrices de tolérance, elles, ne dépendent que des
tirages : mêmes options, mêmes résultats.
