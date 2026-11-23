# 🧬 ADN Data Storage

**Une boîte à outils Rust pour le stockage de données sur ADN : les schémas de la littérature,
les codes correcteurs, la simulation, l'outillage — rien n'est annoncé sans être mesuré.**

ADN Data Storage transforme un fichier numérique en séquences d'ADN, et le restitue à
l'identique. Le fichier devient un ensemble d'oligos, écrit dans un fichier FASTA qu'on peut
confier à un prestataire de synthèse ; les lectures issues du séquençage redonnent le fichier
d'origine.

C'est une bibliothèque Rust, une ligne de commande (`adn`) et un serveur web (`adn-web`). Le
projet vise à couvrir les grandes idées de la littérature du stockage sur ADN — des schémas
d'encodage publiés aux familles de codes correcteurs. L'état de chaque brique, livrée ou au
programme, est dans [la couverture de la littérature](#-couverture-de-la-littérature).

> **Périmètre** : c'est un logiciel d'encodage et de décodage. La synthèse et le séquençage sont
> confiées à un prestataire : le logiciel écrit le fichier FASTA à lui envoyer et lit les lectures
> qu'il renvoie. Les adaptateurs de bibliothèque (Illumina) sont au programme, pas encore dans le
> code.

---

## 📋 Table des matières

- [Pourquoi l'ADN ?](#-pourquoi-ladn-)
- [Couverture de la littérature](#-couverture-de-la-littérature)
- [Ce que fait la bibliothèque](#-ce-que-fait-la-bibliothèque)
- [Architecture](#-architecture)
- [Installation et usage](#-installation-et-usage)
- [Spécifications techniques](#-spécifications-techniques)
- [Résultats des tests](#-résultats-des-tests)
- [Performances mesurées](#-performances-mesurées)
- [Limites connues](#-limites-connues)
- [Feuille de route](#-feuille-de-route)
- [Contribuer](#-contribuer)
- [Ressources](#-ressources)
- [Licence](#-licence)

---

## 🚀 Pourquoi l'ADN ?

L'ADN est un support attrayant pour l'archivage à long terme, par sa densité et sa durabilité.

### Le problème

- Le volume mondial de données continue de croître exponentiellement
- Les supports classiques (disques, bandes, SSD) durent 5 à 10 ans et exigent des migrations
  constantes
- Les centres de données consomment beaucoup d'électricité, notamment pour le refroidissement

### L'ADN comme support

- **Densité** : théoriquement ~455 Po par gramme (2 bits par base, ~1,4 × 10²¹ bases par gramme)
- **Durabilité** : stable pendant des siècles à des millénaires au frais, au sec et à l'obscurité
- **Sans énergie** : une fois synthétisé, l'ADN ne consomme rien pour conserver ses données

> **Note** : ces chiffres viennent de la littérature scientifique ; ce sont des propriétés des
> molécules, pas des mesures de ce logiciel. La bibliothèque s'occupe de l'étape
> **encodage/décodage** — convertir des octets ↔ séquences d'ADN. La densité et le coût réels
> dépendent du prestataire de synthèse et de séquençage.

---

## 🗺️ Couverture de la littérature

Le stockage sur ADN a produit, en dix ans, une poignée d'idées fortes. ADN Data Storage veut les
rassembler dans une seule boîte à outils — et dire où chacune en est.

| Famille | Brique | Source | Statut |
|---------|--------|--------|:------:|
| Encodage | DNA Fountain : gouttes LT sans adresse, criblage | Erlich et Zielinski, 2017 | ✅ livré |
| Encodage | Brins codés : index dans l'ADN, deux niveaux de Reed-Solomon | architecture de Grass et al., 2015 | ✅ livré |
| Encodage | Code ternaire sans répétition, parité XOR | Goldman et al., 2013 | 📋 planifié |
| Encodage | Reed-Solomon sur le corps à 47 éléments | Grass et al., 2015 | 📋 planifié |
| Encodage | Code à hash, résistant aux insertions et suppressions (HEDGES) | Press et al., 2020 | 📋 planifié |
| Codes correcteurs | Reed-Solomon GF(256), interne et externe | — | ✅ livré |
| Codes correcteurs | Code LT, marge de perte certifiée | Luby, 2002 | ✅ livré |
| Codes correcteurs | Code convolutif et décodage de Viterbi (K = 7, rendement ½) | — | 📋 planifié |
| Codes correcteurs | LDPC à propagation de croyances | — | 📋 planifié |
| Codes correcteurs | Entrelacement contre les salves d'erreurs | — | 📋 planifié |
| Traitement | Compression (zstd, LZ4), manifeste embarqué, empreinte SHA-256 | — | ✅ livré |
| Format | FASTA | — | ✅ livré |
| Format | Bibliothèques Illumina : adaptateurs P5/P7, index, multiplexage | — | 📋 planifié |
| Simulation | Synthèse, séquençage, perte, couverture — reliée au décodeur | — | ✅ livré |
| Outillage | CLI, serveur web, SQLite et PostgreSQL, Docker, CI | — | ✅ livré |

✅ livré et mesuré · 📋 planifié : annoncé ici comme objectif, pas encore dans le code

---

## ✨ Ce que fait la bibliothèque

### Les trois garanties

1. **Rien n'est rendu sans vérification.** L'empreinte SHA-256 du fichier voyage dans l'archive.
   Un fichier n'est écrit que si l'empreinte du contenu reconstitué est identique. Une archive
   abîmée produit une erreur, jamais un contenu faux.
2. **Une archive se suffit à elle-même.** Elle se décode depuis ses seules séquences : ni en-tête
   FASTA, ni nom de fichier, ni paramètre extérieur. Le schéma, la compression et les paramètres
   de codage sont portés par des oligos « manifestes ».
3. **Les oligos respectent les contraintes biochimiques.** Par défaut 45 à 55 % de GC, et jamais
   plus de trois bases identiques de suite.

### Les schémas d'encodage

| Schéma | Idée | Statut |
|--------|------|:------:|
| DNA Fountain — `fountain` | gouttes sans adresse, réglables par la marge de perte | ✅ livré |
| Brins codés — `strands` | brins adressés, protégés par deux niveaux de Reed-Solomon | ✅ livré |
| Goldman 2013 | code ternaire, base toujours différente de la précédente, parité XOR | 📋 planifié |
| Grass 2015 | Reed-Solomon sur le corps à 47 éléments, un symbole par groupe de bases | 📋 planifié |
| HEDGES — Press et al., 2020 | code à hash, résistant aux insertions et aux suppressions | 📋 planifié |

Les deux schémas livrés, comparés :

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

### Corrections d'erreurs

| Code | Où | Corrige | Statut |
|------|----|---------|:------:|
| Reed-Solomon interne | chaque oligo `fountain` (2 octets par défaut, réglable jusqu'à 32) | ⌊parité/2⌋ octets erronés | ✅ livré, mesuré |
| Code LT + épluchage | entre les oligos `fountain` | la marge de perte déclarée, certifiée à l'encodage | ✅ livré, mesuré |
| Reed-Solomon interne | chaque brin `strands` (4 octets) | 2 octets erronés | ✅ livré, mesuré |
| Reed-Solomon externe | chaque bloc de 255 brins `strands` (32 brins de parité) | tout motif `2 × erreurs + brins manquants ≤ 32` | ✅ livré, mesuré |
| Code concaténé | `strands` : Reed-Solomon interne sous Reed-Solomon externe | les deux codes à la fois | ✅ livré |
| Code convolutif + Viterbi | code interne (K = 7, rendement ½), décodage par treillis | erreurs dispersées, avec des décisions souples | 📋 planifié |
| LDPC | code externe à rendement élevé, propagation de croyances | une fraction d'erreurs aléatoires proche de la capacité du canal | 📋 planifié |
| Entrelacement | avant Reed-Solomon | transforme une salve en erreurs dispersées | 📋 planifié |

Les mesures de tolérance (670 tirages de simulation, erreurs réelles injectées) sont dans
[les schémas d'encodage](docs/encoding_schemes.md#tolérance-aux-erreurs).

### Traitement des données

- **Compression** : zstd s'il rapporte, sinon aucune (LZ4 au choix) ; le choix est inscrit dans
  l'archive, le décodeur n'a rien à deviner
- **Manifeste** : 12 à 64 oligos porteurs des paramètres, votés à la majorité entre lectures
- **Criblage biochimique** : chaque oligo produit respecte les contraintes, sans exception
- **Marge certifiée** : l'encodeur tient un décodeur à jour et ne livre l'archive que si elle reste
  décodable après des pertes aléatoires, sur 256 tirages indépendants

### Format et conformité

- **FASTA** : lecture et écriture du format standard, séquences repliées à 60 nt ; les en-têtes
  sont informatifs, le décodage ne les lit pas
- **Illumina** 📋 : adaptateurs P5/P7, index i5/i7 et multiplexage — prévu ; l'ancien code n'en
  contenait que des séquences factices
- **Contraintes** : bornes de GC et homopolymère configurables par encodage ; des contraintes
  irréalisables sont refusées d'emblée, avec le calcul qui le justifie
- **Format autosuffisant** : décrit précisément dans [docs/FORMAT.md](docs/FORMAT.md)

### Simulation

- **Canal** : substitutions, insertions et suppressions de séquençage ; erreurs de synthèse,
  portées par toutes les lectures d'un oligo ; perte d'oligos ; couverture de Poisson
- **Reproductible** : générateur déterministe à graines explicites ; un tirage rejoué donne le
  même résultat
- **Bilan** : récupérations, échecs annoncés, fichiers faux — `adn simulate` les compte

### CLI et serveur web

- **CLI** (`adn`) : `encode`, `decode`, `verify`, `simulate`, `visualize` (table, JSON ou HTML),
  sortie `--json` partout
- **Web** (`adn-web`) : dépôt de fichier par glisser-déposer, tâches d'encodage et de décodage,
  téléchargement des résultats. **Pas d'authentification** : réservé à un usage local

---

## 🏗️ Architecture

### Structure du workspace

```
DNAproof/
├── crates/
│   ├── core/         # format v2, schémas fountain et strands, enveloppe, FASTA, vérification
│   ├── cli/          # ligne de commande (clap)
│   ├── web/          # serveur local (actix-web + tera), tâches, téléchargements
│   ├── storage/      # miroir des archives : SQLite ou PostgreSQL (sqlx)
│   └── simulation/   # canal d'erreurs et métriques
├── docs/             # FORMAT.md, schémas, performances, API web
├── Cargo.toml        # manifeste du workspace (verrou suivi dans le dépôt)
└── config.toml       # configuration du serveur, documentée ligne à ligne
```

### Pipeline d'encodage

```
Fichier d'entrée
    │
    ▼
┌───────────────────┐
│ Enveloppe (60 o)  │ compression (zstd s'il rapporte), longueurs,
│                   │ SHA-256 du fichier, CRC-32
└──────┬────────────┘
       │  corps
       ▼
┌───────────────────┐
│ Manifeste         │ 12 à 64 oligos « ADNF » : schéma, compression,
│                   │ longueur du corps, paramètres du schéma
└──────┬────────────┘
       │  corps réparti selon le schéma
       ├────────────────────────────┐
       ▼                            ▼
┌───────────────────┐        ┌───────────────────┐
│ fountain          │        │ strands           │
│ blocs de 32 o →   │        │ tranches de 30 o →│
│ gouttes LT        │        │ brins indexés,    │
│ (soliton robuste) │        │ RS interne 4 o,   │
│ + graine 4 o      │        │ RS externe 32     │
│ + RS 2 o          │        │ brins / bloc 223  │
└──────┬────────────┘        └──────┬────────────┘
       │ chaque candidat            │ chaque brin
       ▼                            ▼
┌───────────────────────────────────────┐
│ Criblage : GC 45–55 %, suites ≤ 3     │
│ (le candidat qui viole est écarté)    │
└──────┬────────────────────────────────┘
       ▼
  Octets → ADN (2 bits par nucléotide) → FASTA
```

À l'encodage `fountain`, un décodeur tourne en même temps que l'encodeur : des gouttes sont
ajoutées jusqu'à ce que l'archive survive à 256 tirages de perte aléatoire au niveau de la marge
demandée.

### Pipeline de décodage (en sens inverse)

```
Lectures FASTA → normalisation (majuscules, blancs) → manifeste (vote entre lectures)
→ corps selon le schéma (épluchage LT, ou code interne puis externe)
→ contrôle CRC-32 → décompression → contrôle SHA-256 → fichier original
```

Si le corps ne passe pas le contrôle, DNA Fountain rejoue le décodage sans les lectures que la
parité a « corrigées » : elles peuvent l'avoir été à tort. Si l'empreinte finale ne correspond
pas, rien n'est écrit.

---

## 🧪 Installation et usage

### Prérequis

- Rust **1.94** ou plus récent ([rustup](https://rustup.rs)) — imposé par `sqlx 0.9`
- Facultatif : Docker pour l'image, un serveur PostgreSQL pour le miroir des archives

### Construire

```sh
git clone https://github.com/duan78/DNAproof
cd DNAproof
cargo build --release --locked -p adn-cli -p adn-web
```

`target/release/adn` est la ligne de commande ; `target/release/adn-web`, le serveur web (sur
Windows, les exécutables ont l'extension `.exe`).

### CLI

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

Les options principales :

| Option | Rôle |
|--------|------|
| `-a, --algorithm <fountain\|strands>` | schéma d'encodage (`fountain` par défaut) |
| `-r, --redundancy <r>` | marge de perte de DNA Fountain (`1.1` par défaut) |
| `--rs-parity <1-32>` | octets de parité par oligo, pour DNA Fountain (`2`) : à relever devant des erreurs de synthèse |
| `-c, --compression <auto\|none\|lz4\|zstd>` | compression du fichier avant encodage |
| `--gc-min`, `--gc-max`, `--max-homopolymer` | contraintes biochimiques (0.45, 0.55, 3) |
| `--json` | sortie en JSON, pour un script |

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
concatènent (`--input` se répète). `--expect-sha256` impose l'empreinte du fichier : en cas
d'écart, rien n'est écrit.

`adn verify` contrôle une archive sans rien écrire ; `adn simulate` dégrade une archive (perte,
substitutions, insertions, suppressions, couverture) et compte les récupérations sur N tirages ;
`adn visualize` donne les statistiques d'un ensemble de séquences (table, JSON ou HTML).

### Serveur web

```sh
cd crates/web
../../target/release/adn-web
# Serveur ADN 0.2.0 sur http://127.0.0.1:8080 (fichiers jusqu'à 25 Mio, 2 tâche(s) à la fois)
```

On dépose un fichier, on télécharge l'archive FASTA ; ou l'inverse. Les résultats sont conservés
24 heures. Le serveur n'écoute que sur la machine elle-même, et il n'y a pas d'authentification :
c'est un outil de poste de travail. Pour l'exposer, une base de données, un proxy inverse, systemd
ou Docker : [guide de production](PRODUCTION_GUIDE.md). L'API : [référence](docs/api_reference.md).

### Bibliothèque

```rust
let options = adn_core::EncodeOptions::for_scheme(adn_core::Scheme::Strands);
let archive = adn_core::encode(b"donnees", &options).unwrap();
adn_core::fasta::write_archive(
    "archive.fasta",
    &archive.oligos,
    adn_core::Scheme::Strands,
    archive.stats.manifest_copies,
).unwrap();

let records = adn_core::fasta::read_file("archive.fasta").unwrap();
let decoded = adn_core::decode(
    records.iter().map(|record| record.sequence.as_slice()),
    &adn_core::DecodeOptions::default(),
).unwrap();
assert_eq!(decoded.data, b"donnees");
```

### Lancer les tests

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo bench -p adn-core
cargo audit          # aucune vulnérabilité connue des dépendances
```

Le test du dépôt PostgreSQL est ignoré par défaut : il lui faut un serveur et
`STORAGE_TEST_DATABASE_URL=postgres://…`. Les tests d'échelle (jusqu'à 100 Mio) aussi :
`cargo test --release -p adn-core --test scale -- --ignored`.

---

## 🧬 Spécifications techniques

### Contraintes biochimiques (configurables)

- **GC** : 45–55 % par défaut, bornes configurables de 0 à 1
- **Homopolymère** : au plus 3 bases identiques de suite par défaut
- **Criblage** : tout oligo qui viole les contraintes est écarté avant écriture — jamais accepté
  « pour ne pas bloquer »
- **Contraintes irréalisables** : refusées d'emblée, avec la probabilité calculée qu'un oligo au
  hasard les respecte

### DNA Fountain (Erlich et Zielinski, 2017)

- **Algorithme** : codes LT, distribution soliton robuste (`c` ajusté automatiquement aux petites
  archives, `δ = 0,001`), tirages rejouables par un générateur figé dans le format
- **Oligo** : graine de 4 octets + charge utile masquée de 32 octets + parité Reed-Solomon
  (2 octets par défaut) = 152 nt
- **Marge de perte** : certifiée par l'encodeur sur 256 tirages indépendants (96 puis 32 au-delà
  de 50 000 et 500 000 blocs)
- **Décodage** : épluchage par gouttes certaines d'abord — les lectures reçues sans erreur votent,
  car trois substitutions bien placées peuvent former un autre mot de code que la parité ne
  détecte pas ; second passage sans les lectures corrigées si le contrôle d'intégrité échoue

### Brins codés (architecture de Grass et al., 2015)

- **Oligo** : nonce de 3 octets + index de 3 octets + charge utile de 30 octets + parité interne
  de 4 octets = 160 nt
- **Code interne** : Reed-Solomon par brin, corrige 2 octets erronés par lecture
- **Code externe** : 32 brins de parité par bloc de 223, colonne par colonne ; contrôle et
  correction de chaque bloc (un brin faux compte comme deux brins manquants)
- **Limite** : l'index tient sur 24 bits, d'où le corps maximal de 420 Mio

### Enveloppe

| décalage | taille | contenu |
|----------|--------|---------|
| 0 | 4 | magic `ADN2` |
| 4 | 1 | version du format (2) |
| 5 | 1 | compression : 0 aucune, 1 LZ4, 2 zstd |
| 8 | 8 | longueur du fichier original |
| 16 | 8 | longueur des données stockées |
| 24 | 32 | SHA-256 du fichier original |
| 56 | 4 | CRC-32 de l'en-tête suivi des données |
| 60 | … | données (compressées ou non) |

Le détail complet (manifeste, masquage, générateur pseudo-aléatoire, limites) est dans
[docs/FORMAT.md](docs/FORMAT.md).

---

## 📊 Résultats des tests

```sh
cargo test --workspace --locked
```

| Catégorie | Nombre | Ce qui est vérifié |
|-----------|:------:|--------------------|
| Cœur, unitaires | 102 | enveloppe, manifeste, gouttes, brins, Reed-Solomon, contraintes, générateur |
| Aller-retour format | 10 | `encode → FASTA → decode == original` pour les deux schémas, en-têtes renommés ou perdus, CRLF et minuscules, contraintes strictes |
| Robustesse | 8 | substitutions corrigées, marge honorée puis refusée au-delà, brins manquants réparés, manifestes abîmés, altérations au hasard : jamais un fichier faux |
| Propriétés (proptest) | 2 | aller-retour sur entrées au hasard (schéma × type × taille × mélange) et à contraintes aléatoires |
| Simulation, bout en bout | 12 | encodage → canal d'erreurs → décodage, 12 scénarios de canal |
| Simulation, unitaires | 9 | modèle du canal, statistiques |
| Stockage | 13 | SQLite et PostgreSQL : lots, transactions, reprise après redémarrage |
| CLI | 16 | les cinq sous-commandes exécutées comme par un utilisateur |
| Serveur web | 38 | pages, API, refus, en-têtes, reprise, hôtes |
| Exemple du README | 1 | le code montré plus haut, tel quel |
| Exemple de la bibliothèque | 1 | le doc-test de `adn-core` |
| **Total** | **212** | **0 échec** (+ 5 ignorés : 4 tests d'échelle, 1 PostgreSQL sans serveur) |

### Propriétés clés

- **Tous les aller-retours sont stricts** : `assert_eq!(original, decoded)` — aucun raccourci du
  genre « vérifier que ce n'est pas vide »
- **Les contraintes sont vérifiées oligo par oligo**, pas échantillonnées
- **Un fichier faux n'a jamais été produit** : sur les 670 tirages de la matrice de tolérance et
  les tests d'altération au hasard, chaque contenu reconstitué était exact, et chaque archive trop
  abîmée a été refusée avec un message
- **Le canal de simulation est branché sur le décodeur** : les tests de bout en bout vérifient la
  récupération réelle, pas seulement un aller-retour sans bruit

---

## 📈 Performances mesurées

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

---

## ⚠️ Limites connues

- **Pas de vraie synthèse** : logiciel uniquement ; aucune interface avec du matériel ou des
  prestataires de synthèse et de séquençage
- **Pas d'authentification dans le serveur web** : usage local ; ajouter une authentification
  devant (proxy) pour tout autre usage
- **Erreurs de lecture** : avec sa parité par défaut (deux octets), DNA Fountain ne corrige qu'une
  erreur par oligo : dès 0,1 % de substitution par nucléotide, il faut plusieurs lectures par
  oligo, et une vingtaine à 1 %. `strands` corrige deux octets par brin, et jusqu'à 32 brins
  manquants sur 255
- **Insertions et suppressions** : une lecture qui change de longueur est écartée. À 0,5 %
  d'insertion et de suppression, `fountain` ne s'en tire qu'à marge large, et pas à tous les
  coups ; `strands` jamais
- **Erreurs de synthèse** : portées par toutes les lectures d'un oligo, elles ne se compensent
  pas : il faut plus de parité par oligo (`--rs-parity 8`)
- **Petits fichiers** : DNA Fountain leur donne beaucoup de redondance (141 oligos pour 75
  octets) ; `strands` est bien plus économe
- **Fichiers de plus de 420 Mio** : seul `fountain` convient, et l'encodage demande environ
  cinquante fois la taille du fichier en mémoire
- **Le serveur web n'expose ni la marge ni la parité** : passer par la ligne de commande pour un
  canal difficile
- **Tout n'est pas encore reconstruit** : Goldman 2013, Grass 2015, le code convolutif et Viterbi,
  le LDPC, l'entrelacement et le format Illumina sont au programme, pas encore dans le code —
  voir [la couverture de la littérature](#-couverture-de-la-littérature)

---

## 🚀 Feuille de route

### État actuel : v0.2.0

- ✅ Format v2 autosuffisant : manifeste embarqué, empreinte SHA-256 vérifiée à chaque décodage
- ✅ Deux schémas (`fountain`, `strands`) à aller-retour strict, contraintes biochimiques garanties
- ✅ Marge de perte certifiée à l'encodage ; décodeurs durcis (vote par goutte, code externe par
  bloc)
- ✅ Simulation réaliste (synthèse, séquençage, perte, couverture) branchée sur le décodeur
- ✅ Matrice de tolérance mesurée : 670 tirages, aucun fichier faux
- ✅ CLI complète (`encode`, `decode`, `verify`, `simulate`, `visualize`, `--json`)
- ✅ Serveur web avec suivi de tâches, reprise après redémarrage, base SQLite ou PostgreSQL
- ✅ CI au vert : format, Clippy, tests (Ubuntu et Windows), MSRV 1.94, PostgreSQL, audit des
  dépendances, image Docker
- ✅ Documentation refaite sur des mesures : [format](docs/FORMAT.md),
  [schémas](docs/encoding_schemes.md), [performances](docs/PERFORMANCE.md),
  [production](PRODUCTION_GUIDE.md), [API web](docs/api_reference.md)

### Le programme

La direction du projet : couvrir, pour de vrai, ce que la littérature a proposé de meilleur.

1. **Les schémas publiés** — Goldman 2013 (code ternaire) et Grass 2015 (Reed-Solomon sur
   GF(47)), conformes aux articles, avec mesure de tolérance.
2. **Le format des bibliothèques Illumina** — adaptateurs P5/P7, index i5/i7, multiplexage et
   démultiplexage tolérants aux erreurs.
3. **Les familles de codes correcteurs** — code convolutif et décodage de Viterbi, LDPC à
   propagation de croyances capable de corriger, entrelacement contre les salves d'erreurs.
4. **Les insertions et suppressions** — le point faible mesuré : HEDGES et codes apparentés.

Ensuite :

- [ ] Marge et parité réglables depuis le serveur web (aujourd'hui : ligne de commande)
- [ ] Envoi en flux de gros fichiers (aujourd'hui : le fichier est lu en mémoire)
- [ ] Authentification pour publier le serveur web au-delà d'un proxy
- [ ] Conformité aux spécifications d'industrie de la DNA Data Storage Alliance
- [ ] Intégration d'un prestataire de synthèse et de séquençage (API)
- [ ] Plus loin : turbocodes et codes polaires, accélération GPU

---

## 🤝 Contribuer

Les contributions sont bienvenues. C'est un projet de recherche et d'éducation.

```sh
git checkout -b feature/votre-fonction
# … coder …
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git commit -m "feat: description de la fonction"
git push origin feature/votre-fonction
```

- Ajouter des tests aux nouvelles fonctions (aller-retours stricts de préférence)
- Toute erreur doit être annoncée : un fichier n'est jamais rendu sans vérification
- Documenter tout nouveau schéma d'encodage, et le mesurer (`adn simulate`, `cargo bench`)

---

## 📚 Ressources

### Documentation du projet

- [Spécification du format d'archive](docs/FORMAT.md)
- [Les schémas d'encodage et leur tolérance mesurée](docs/encoding_schemes.md)
- [Performances mesurées](docs/PERFORMANCE.md)
- [Guide de production](PRODUCTION_GUIDE.md) · [Référence de l'API web](docs/api_reference.md)
- [Journal des changements](CHANGELOG.md)

### Articles de recherche

- Erlich Y., Zielinski D. — *DNA Fountain enables a robust and efficient storage architecture*,
  Science 355(6328):950–952, 2017.
  [doi:10.1126/science.aaj2038](https://www.science.org/doi/10.1126/science.aaj2038)
- Grass R. N., Heckel R., Puddu M., Paunescu D., Stark W. J. — *Robust Chemical Preservation of
  Digital Information on DNA in Silica with Error-Correcting Codes*, Angewandte Chemie
  International Edition 54:2552–2555, 2015.
  [doi:10.1002/anie.201411578](https://onlinelibrary.wiley.com/doi/10.1002/anie.201411578)
- Goldman N. et al. — *Towards practical, high-capacity, low-maintenance information storage in
  synthesised DNA*, Nature 494:77–80, 2013.
  [doi:10.1038/nature11875](https://www.nature.com/articles/nature11875)
- Luby M. — *LT codes*, FOCS 2002.
- Press W. H., Hawkins J. A., Jones S. K. Jr., Schaub J. M., Finkelstein I. J. — *HEDGES
  error-correcting code for DNA storage corrects indels and allows sequence constraints*,
  Proceedings of the National Academy of Sciences 117:18489–18496, 2020 (au programme du projet).
  [doi:10.1073/pnas.2004821117](https://www.pnas.org/doi/10.1073/pnas.2004821117)

### Industrie

- [DNA Data Storage Alliance](https://dnastoragealliance.org/)
- [Twist Bioscience](https://www.twistbioscience.com/) — synthèse d'ADN
- [Illumina](https://www.illumina.com/) — séquençage

---

## 📄 Licence

**MIT OU Apache-2.0**, au choix : [`LICENSE-MIT`](LICENSE-MIT) ou
[`LICENSE-APACHE`](LICENSE-APACHE).

---

<div align="center">

**[GitHub](https://github.com/duan78/DNAproof)** ·
[Problèmes](https://github.com/duan78/DNAproof/issues) ·
[Discussions](https://github.com/duan78/DNAproof/discussions)

*Écrit en Rust · v0.2.0*

</div>
