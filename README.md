# 🧬 ADN Data Storage

**Archiver des fichiers numériques en séquences d'ADN, et les restituer à l'identique.**

Une bibliothèque Rust, une ligne de commande (`adn`) et un serveur web (`adn-web`). Le fichier
devient un ensemble d'oligos, écrit dans un fichier FASTA qu'on peut confier à un prestataire de
synthèse ; les lectures issues du séquençage redonnent le fichier d'origine.

> **Périmètre.** C'est un logiciel d'encodage et de décodage. Il ne synthétise ni ne séquence
> d'ADN : il produit des fichiers FASTA, et il lit des fichiers de lectures.

## Trois garanties

1. **Rien n'est rendu sans vérification.** L'empreinte SHA-256 du fichier voyage dans l'archive.
   Un fichier n'est écrit que si l'empreinte du contenu reconstitué est identique. Une archive
   abîmée produit une erreur, jamais un contenu faux.
2. **Une archive se suffit à elle-même.** Elle se décode depuis ses seules séquences : ni en-tête
   FASTA, ni nom de fichier, ni paramètre extérieur. Le schéma, la compression et les paramètres
   de codage sont portés par des oligos « manifestes ».
3. **Les oligos respectent les contraintes biochimiques.** Par défaut 45 à 55 % de GC, et jamais
   plus de trois bases identiques de suite.

Le format est décrit précisément dans [`docs/FORMAT.md`](docs/FORMAT.md).

## Les deux schémas

| | `fountain` — DNA Fountain | `strands` — brins codés |
|---|---|---|
| Principe | gouttes sans ordre ni adresse : l'archive se reconstitue à partir de n'importe quelles lectures | brins adressés, protégés par deux niveaux de Reed-Solomon |
| Oligo | 152 nt | 160 nt |
| Redondance | réglable : la *marge de perte* `r` garantit la lecture après la perte aléatoire de `1 − 1/r` des oligos | 32 brins de parité par bloc de 223 |
| Limite du format | ≈ 137 Gio | ≈ 420 Mio |
| Densité (fichier incompressible) | 1,41 bit/nt (marge 1,1) ; 1,04 (marge 1,5) | 1,31 bit/nt |
| Encodage de 1 Mio | 0,19 s | 0,04 s |
| Bien pour | les archives qui doivent traverser des pertes ; les très gros fichiers | les petits fichiers ; un encodage rapide |

**Comment choisir.** `fountain` est le schéma par défaut : c'est celui de l'article de référence,
et sa marge de perte se règle. En dessous de quelques centaines de kilooctets, `strands` produit
beaucoup moins d'oligos : un fichier de 75 octets donne 49 brins, contre 141 oligos avec
`fountain`. Au-delà de 420 Mio, seul `fountain` convient.

Le détail, les paramètres et la tolérance aux erreurs mesurée de chaque schéma sont dans
[`docs/encoding_schemes.md`](docs/encoding_schemes.md).

## Installation

Il faut Rust 1.94 ou plus récent ([rustup](https://rustup.rs)).

```sh
git clone https://github.com/duan78/DNAproof
cd DNAproof
cargo build --release --locked -p adn-cli -p adn-web
```

`target/release/adn` est la ligne de commande ; `target/release/adn-web`, le serveur web. Une
image Docker est décrite dans le [guide de production](PRODUCTION_GUIDE.md#docker).

## Ligne de commande

### Encoder

```console
$ adn encode --input lettre.txt
Encodage de lettre.txt :
  Fichier     : lettre.txt (75 o)
  Schéma      : DNA Fountain (Erlich et Zielinski, 2017)
  Compression : aucune (corps de 135 o)
  Oligos      : 141 au total : 129 de données et 12 manifestes, de 152 nt chacun
  Nucléotides : 21 432
  Densité     : 0.028 bits par nucléotide
  Redondance  : marge 1.10 ; 25.80 gouttes par bloc
  SHA-256     : 2b8fe8fb37d80884a8a24aa8e419f526cdb665659746b2653b5fa69b23ae4b78
  Durée       : 0.01 s

Archive écrite : lettre.fasta
```

L'archive s'écrit à côté du fichier, ou là qu'indique `--output`. Les options :

| Option | Rôle |
|--------|------|
| `-a, --algorithm <fountain\|strands>` | schéma d'encodage (`fountain` par défaut) |
| `-r, --redundancy <r>` | marge de perte de DNA Fountain (`1.1` par défaut) |
| `--rs-parity <1-32>` | octets de parité Reed-Solomon par oligo, pour DNA Fountain (`2`) : un oligo résiste à la moitié d'octets erronés, et s'allonge de 4 nt par octet. À relever devant des erreurs de synthèse |
| `-c, --compression <auto\|none\|lz4\|zstd>` | compression du fichier avant encodage (`auto` : zstd s'il rapporte) |
| `--gc-min`, `--gc-max`, `--max-homopolymer` | contraintes biochimiques (0.45, 0.55, 3) |
| `--json` | sortie en JSON, pour un script |

### Décoder

```console
$ adn decode --input lettre.fasta
Décodage de lettre.fasta :
  Schéma         : DNA Fountain (Erlich et Zielinski, 2017)
  Compression    : aucune
  Lectures       : 141 (141) ; 12 manifestes confirmants
  Corrigées      : 0 lectures
  Irrécupérables : 0 lectures
  Fichier        : lettre.decoded (75 o)
  SHA-256        : 2b8fe8fb37d80884a8a24aa8e419f526cdb665659746b2653b5fa69b23ae4b78

L'empreinte embarquée dans l'archive correspond.
```

Le schéma et la compression sont lus dans l'archive. Plusieurs fichiers de lectures se
concatènent (`--input` se répète). `--expect-sha256` impose l'empreinte du fichier :

```console
$ adn decode --input archive.fasta --expect-sha256 0000…0000
Error: l'empreinte du fichier restitué (976d9a67…) ne correspond pas à celle attendue (0000…) ; rien n'a été écrit
```

### Vérifier une archive

`adn verify` contrôle une archive sans rien écrire : contraintes de chaque oligo, manifeste,
décodabilité, empreinte.

```console
$ adn verify --input alea-brins.fasta
Vérification de alea-brins.fasta :
  Séquences   : 40 043 (alea-brins.fasta), de 160 à 160 nt
  GC          : de 45.0 à 55.0 %, 50.2 % en moyenne
  Contraintes : 40 043/40 043 séquences respectent GC 45–55 % et suites ≤ 3 (la plus longue : 3)
  Archive     : décodable ; empreinte vérifiée
  Schéma      : Brins codés (Reed-Solomon interne et externe)
  Fichier     : 1 048 576 octets
  SHA-256     : 976d9a6758b4c379fc7422ce2658a8fb235c3ab8e20805595d8a4e70b72e9419
```

### Simuler la dégradation d'une archive

`adn simulate` dégrade une archive — perte d'oligos, erreurs de synthèse portées par toutes les
lectures d'un oligo, erreurs de séquençage propres à chaque lecture, couverture — puis tente de
la relire, plusieurs fois :

```console
$ adn simulate --archive --input alea-fontaine.fasta --substitution-rate 0.001 --coverage 3 --trials 10
Simulation sur alea-fontaine.fasta (archive existante) : 10 tirages, schéma DNA Fountain (Erlich et Zielinski, 2017)
Canal, par nucléotide : synthèse 0.00 % / 0.00 % / 0.00 %, séquençage 0.10 % / 0.00 % / 0.00 % (substitution / insertion / délétion) ; perte 0.00 % ; couverture 3

Tirage   Lectures Oligos perdus   Intactes   Hors long.  Résultat
1          159619         5.1 %     137215            0  récupéré (second passage)
…
Récupérations : 10 sur 10 tirages
Fichiers faux : 0 : un échec est toujours annoncé
```

Sans `--archive`, le fichier d'origine est encodé puis dégradé. `--seed` rend un tirage
reproductible.

### Statistiques de séquences

```console
$ adn visualize --input lettre.fasta
Séquences : 141   Nucléotides : 21432   Longueurs : 152 à 152 nt
GC : 45.4 à 54.6 % (moyenne 49.9 %)   Plus longue suite : 3   Conformes : 141/141

Rang   Longueur        GC Suite max Conforme
0           152     49.3 %         3 oui
…
```

`--format html` écrit une page, `--format json` un objet, `--all` toutes les séquences.

## Serveur web

```sh
cd crates/web
../../target/release/adn-web
# Serveur ADN 0.2.0 sur http://127.0.0.1:8080 (fichiers jusqu'à 25 Mio, 2 tâche(s) à la fois)
```

On dépose un fichier, on télécharge l'archive FASTA ; ou l'inverse. Les résultats sont conservés
24 heures. Le serveur n'écoute que sur la machine elle-même, et il n'y a pas d'authentification :
c'est un outil de poste de travail. Pour l'exposer, une base de données, un proxy inverse, systemd
ou Docker : [guide de production](PRODUCTION_GUIDE.md). L'API : [référence](docs/api_reference.md).

## Bibliothèque

```rust
use adn_core::{decode, encode, DecodeOptions, EncodeOptions, Scheme};

let options = EncodeOptions::for_scheme(Scheme::Strands);
let archive = encode(b"donnees", &options)?;
adn_core::fasta::write_archive("archive.fasta", &archive.oligos, Scheme::Strands, archive.stats.manifest_copies)?;

let records = adn_core::fasta::read_file("archive.fasta")?;
let decoded = decode(records.iter().map(|record| record.sequence.as_slice()), &DecodeOptions::default())?;
assert_eq!(decoded.data, b"donnees");
```

Les crates : `adn-core` (codecs, format, FASTA), `adn-simulation` (canal et erreurs),
`adn-storage` (SQLite, PostgreSQL), `adn-cli`, `adn-web`.

## Performances mesurées

`cargo bench -p adn-core`. Construction `--release`, sur un AMD Ryzen AI Max+ 395 (32 threads).
Fichiers incompressibles — le détail, les grandes tailles et la mémoire dans
[Performances](docs/PERFORMANCE.md) :

| | 64 Kio | 256 Kio | 1 Mio |
|---|---|---|---|
| Encodage `fountain` (marge 1,1) | 32 ms | 70 ms | 187 ms |
| Encodage `strands` | 33 ms | 34 ms | 41 ms |
| Décodage `fountain`, archive intacte | 3,8 ms | 13,7 ms | 63 ms |
| Décodage `strands`, archive intacte | 2,8 ms | 9,0 ms | 31 ms |
| Décodage `fountain`, 0,1 % de substitution, 3 lectures par oligo | 7,0 ms | 26 ms | 113 ms |
| Décodage `strands`, 0,5 % de substitution | 4,7 ms | 12 ms | 45 ms |

En grand : 32 Mio s'encodent en 6,9 s avec `fountain` (1,6 Go de mémoire), en 0,9 s avec
`strands` (0,4 Go). Sur un fichier texte, la compression compte davantage que le schéma : 1,32 Mio
de texte tient en 9 982 oligos une fois compressé par zstd, contre 39 000 pour un Mio d'octets au
hasard.

## Limites connues

Elles sont mesurées, et détaillées dans [les schémas](docs/encoding_schemes.md#tolérance-aux-erreurs).
Sur les 670 tirages de la matrice, aucun fichier faux n'a jamais été rendu.

- **Erreurs de lecture.** Avec sa parité par défaut (deux octets), DNA Fountain ne corrige qu'une
  erreur par oligo : dès 0,1 % de substitution par nucléotide, il faut plusieurs lectures par
  oligo, et une vingtaine à 1 %. `strands` corrige deux octets par brin, et jusqu'à 32 brins
  manquants sur 255.
- **Insertions et suppressions.** Une lecture qui change de longueur est écartée. À 0,5 %
  d'insertion et de suppression, `fountain` ne s'en tire qu'à marge large, et pas à tous les
  coups ; `strands` jamais.
- **Erreurs de synthèse.** Portées par toutes les lectures d'un oligo, elles ne se compensent
  pas : il faut plus de parité par oligo (`--rs-parity 8`).
- **Petits fichiers.** DNA Fountain leur donne beaucoup de redondance : 141 oligos pour 75 octets.
  `strands` est bien plus économe.
- **Fichiers de plus de 420 Mio.** Seul `fountain` convient.
- **La mémoire.** Encoder avec `fountain` demande environ cinquante fois la taille du fichier
  (1,6 Go pour 32 Mio) : la limite pratique d'un poste de travail vient avant celle du format.
  Voir [performances](docs/PERFORMANCE.md).

## Vérifier

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo bench -p adn-core
cargo audit          # aucune vulnérabilité connue des dépendances
```

Le test du dépôt PostgreSQL est ignoré par défaut ; il lui faut un serveur et
`STORAGE_TEST_DATABASE_URL=postgres://…`. Les tests d'échelle (des dizaines de méga-octets)
sont ignorés aussi : `cargo test --release -p adn-core --test scale -- --ignored`.

## Documentation

- [Spécification du format d'archive](docs/FORMAT.md)
- [Les schémas d'encodage](docs/encoding_schemes.md) et [performances mesurées](docs/PERFORMANCE.md)
- [Guide de production](PRODUCTION_GUIDE.md) et [référence de l'API web](docs/api_reference.md)
- [Journal des changements](CHANGELOG.md)

## Références

- Erlich Y., Zielinski D. — *DNA Fountain enables a robust and efficient storage architecture*,
  Science 355:950–954, 2017.
- Grass R. N., Heckel R., Puddu M., Paunescu D., Stark W. J. — *Robust Chemical Preservation of
  Digital Information on DNA in Silica with Error-Correcting Codes*, Angewandte Chemie
  International Edition 54:2552–2555, 2015.
- Luby M. — *LT codes*, FOCS 2002.

## Licence

MIT OU Apache 2.0, au choix : [`LICENSE-MIT`](LICENSE-MIT) ou [`LICENSE-APACHE`](LICENSE-APACHE).
