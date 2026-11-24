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
> confiées à un prestataire : le logiciel écrit le fichier à lui envoyer et lit les lectures
> qu'il renvoie, FASTA ou FASTQ. Les bibliothèques de séquençage sont prises en charge, du pool
> multiplexé au démultiplexage : [Bibliothèques Illumina](docs/LIBRARY.md).

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
| Encodage | Code ternaire de Huffman, jamais deux bases identiques, fenêtres recouvrantes | Goldman et al., 2013 | ✅ livré |
| Encodage | Reed-Solomon sur le corps à 47 éléments, un symbole par trimère | Grass et al., 2015 | ✅ livré |
| Encodage | Code à hachis, résistant aux insertions et suppressions (HEDGES) | Press et al., 2020 | ✅ livré |
| Codes correcteurs | Reed-Solomon GF(256), interne et externe | — | ✅ livré |
| Codes correcteurs | Code LT, marge de perte certifiée | Luby, 2002 | ✅ livré |
| Codes correcteurs | Code convolutif et décodage de Viterbi (K = 7, rendement ½) | Viterbi, 1967 ; Bahl et al., 1974 | ✅ livré |
| Codes correcteurs | LDPC à propagation de croyances | Gallager, 1963 | ✅ livré |
| Codes correcteurs | Décodage séquentiel à pile : la recherche best-first de HEDGES | Zigangirov, 1966 ; Jelinek, 1969 | ✅ livré |
| Codes correcteurs | Entrelacement contre les salves d'erreurs | — | ✅ livré |
| Traitement | Compression (zstd, LZ4), manifeste embarqué, empreinte SHA-256 | — | ✅ livré |
| Format | FASTA et FASTQ | — | ✅ livré |
| Format | Bibliothèques Illumina : adaptateurs P5/P7, index i5/i7, multiplexage | Illumina, *Adapter Sequences* 1000000002694 | ✅ livré |
| Simulation | Synthèse, séquençage, perte, couverture, erreurs en salves — reliée au décodeur | — | ✅ livré |
| Simulation | Séquenceur : lectures appariées, lectures d'index, sauts d'index | — | ✅ livré |
| Outillage | CLI, serveur web, SQLite et PostgreSQL, Docker, CI | — | ✅ livré |
| Encodage | Codes VT et codes à filigrane : d'autres façons de relire les insertions et suppressions | Varshamov et Tenengolts, 1965 ; Davey et MacKay, 2001 | 📋 planifié |
| Codes correcteurs | Turbocodes et codes polaires | Berrou et al., 1993 ; Arıkan, 2009 | 📋 planifié |

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
   plus de trois bases identiques de suite. Une exception, fidèle à son article : le schéma
   `goldman` garantit l'absence de bases répétées, pas la teneur en GC de ses brins.

### Les schémas d'encodage

| Schéma | Idée | Statut |
|--------|------|:------:|
| DNA Fountain — `fountain` | gouttes sans adresse, réglables par la marge de perte | ✅ livré |
| Brins codés — `strands` | brins adressés, protégés par deux niveaux de Reed-Solomon | ✅ livré |
| Goldman 2013 — `goldman` | code ternaire de Huffman, jamais deux bases identiques de suite, chaque base lue dans quatre brins | ✅ livré |
| Grass 2015 — `grass` | Reed-Solomon sur le corps à 47 éléments, un symbole par groupe de trois bases | ✅ livré |
| LDPC et code convolutif — `ldpc` | code convolutif par brin, lu à décisions souples ; LDPC par bloc ; entrelacement | ✅ livré |
| HEDGES — `hedges` | code à hachis par brin, décodé par une recherche best-first : les insertions et les suppressions sont corrigées ; Reed-Solomon en diagonale entre les brins | ✅ livré |

Les six schémas, comparés :

| | `fountain` — DNA Fountain | `strands` — brins codés | `goldman` — Goldman 2013 | `grass` — Grass 2015 | `ldpc` — code convolutif et LDPC | `hedges` — code à hachis (HEDGES), rendement ½ |
|---|---|---|---|---|---|---|
| Principe | gouttes sans ordre ni adresse : l'archive se reconstitue à partir de n'importe quelles lectures | brins adressés, protégés par deux niveaux de Reed-Solomon | un trit par base, jamais deux bases identiques ; chaque base du message lue dans quatre brins, et votée | symboles de GF(47) écrits en trois bases, protégés par deux niveaux de Reed-Solomon | code convolutif par brin, relu à décisions souples ; LDPC par bloc ; entrelacement | chaque base est désignée par un hachis de la position, des bits précédents et du sel ; une recherche best-first resynchronise chaque lecture ; Reed-Solomon en diagonale entre les brins |
| Oligo | 152 nt | 160 nt | 117 nt | 117 nt | 154 nt | 200 ou 300 nt, selon le rendement |
| Redondance | réglable : la *marge de perte* `r` garantit la lecture après la perte aléatoire de `1 − 1/r` des oligos | 32 brins de parité par bloc de 223 | recouvrement ×4 | 14 symboles de parité par brin ; 92 brins de parité par bloc de 713 | code de rendement ½, puis 20 % de brins de parité par bloc de 819 | réglable : le *rendement* du code (¾, ½, ⅓, ¼) ; 32 brins de parité par bloc de 223 |
| Limite du format | ≈ 137 Gio | ≈ 420 Mio | ≈ 2,6 Mo | ≈ 55 Mo | ≈ 173 Mio | ≈ 182 Mio (¼) à ≈ 448 Mio (¾) |
| Densité (fichier incompressible) | 1,41 bit/nt (marge 1,1) ; 1,04 (marge 1,5) | 1,31 bit/nt | 0,34 bit/nt | 0,77 bit/nt ; 1,12 (parité 6) | 0,56 bit/nt ; 0,45 (parité 50) | 1,12 bit/nt (¾) ; 0,70 (½) ; 0,47 (⅓) ; 0,30 (¼) |
| Traverse (mesuré) | 10 % de perte à marge 1,1 | 2 % de substitution | 2 % de substitution à couverture 20 ; 2 % de perte | 5 % de substitution à couverture 3 ; 8 % de perte | 5 % de substitution à couverture 10 ; 10 % de perte ; 5 % d'erreurs de synthèse, une fois sur deux | 4,5 % d'erreurs des trois sortes réunies par base à ½, 15 % à ¼ — 18 % à couverture 20 ; 7 % de perte |
| Encodage de 1 Mio | 0,22 s | 0,05 s | 0,11 s | 0,27 s | 0,17 s | 0,05 s |
| Bien pour | les archives qui doivent traverser des pertes ; les très gros fichiers | les petits fichiers ; un encodage rapide | le schéma de l'article, tel quel ; plusieurs archives dans un même tube | un canal à substitutions denses | un séquençage ou une synthèse très bruités, une salve d'erreurs, quand la densité importe peu | un séquençage à insertions et suppressions — lectures longues et bruitées — ; rien d'autre ne les corrige |

**Comment choisir.** `fountain` est le schéma par défaut : c'est celui de l'article de référence,
et sa marge de perte se règle. En dessous de quelques centaines de kilooctets, `strands` produit
beaucoup moins d'oligos : un fichier de 75 octets donne 49 brins, contre 141 oligos avec
`fountain`. Au-delà de 420 Mio, seul `fountain` convient. `goldman` et `grass` sont les schémas
des articles, pour qui les veut tels quels : le premier n'a pas de code correcteur et demande de
la couverture ; le second est le plus résistant aux substitutions, et le moins dense. `ldpc`
va plus loin que tous devant les erreurs — 5 % de substitution, 5 % d'erreurs de synthèse, 20 %
d'erreurs en salves —, et c'est le seul à lire chaque bit avec son poids de preuve ; il est
moins dense que `strands`, plus lent à décoder, et demande de la couverture. `hedges` est le
seul à corriger les insertions et les suppressions : c'est lui qu'il faut dès que des lectures
changent de longueur, et son rendement se règle à la mesure du canal.

### Corrections d'erreurs

| Code | Où | Corrige | Statut |
|------|----|---------|:------:|
| Reed-Solomon interne | chaque oligo `fountain` (2 octets par défaut, réglable jusqu'à 32) | ⌊parité/2⌋ octets erronés | ✅ livré, mesuré |
| Code LT + épluchage | entre les oligos `fountain` | la marge de perte déclarée, certifiée à l'encodage | ✅ livré, mesuré |
| Reed-Solomon interne | chaque brin `strands` (4 octets) | 2 octets erronés | ✅ livré, mesuré |
| Reed-Solomon externe | chaque bloc de 255 brins `strands` (32 brins de parité) | tout motif `2 × erreurs + brins manquants ≤ 32` | ✅ livré, mesuré |
| Vote par base | entre les quatre brins `goldman` qui couvrent chaque base du message | jusqu'à trois lectures fausses sur quatre, par base | ✅ livré, mesuré |
| Reed-Solomon sur GF(47) | chaque brin `grass` (14 symboles de parité par défaut) | 7 symboles erronés, ou 14 effacements — un trimère hors du jeu des 47 en est un | ✅ livré, mesuré |
| Reed-Solomon sur GF(47²) | chaque bloc de 713 brins `grass` (92 brins de parité) | tout motif `2 × erreurs + brins manquants ≤ 92` | ✅ livré, mesuré |
| Code concaténé | `strands` et `grass` : un code interne par brin sous un code externe par bloc | les deux codes à la fois | ✅ livré |
| Code convolutif | chaque brin `ldpc` : (171, 133) en octal, K = 7, rendement ½, décodé par Viterbi ou par log-MAP | toute salve de 4 bits ; à sorties souples, un rapport de vraisemblance par bit | ✅ livré, mesuré |
| LDPC | chaque bloc de 819 brins `ldpc` (20 % de parité par défaut), propagation de croyances | 10 % des brins manquants ; 22 % avec 50 % de parité | ✅ livré, mesuré |
| Entrelacement | entre les deux, dans `ldpc` | un brin sans lecture devient 104 effacements isolés : à 5 % de brins manquants, 10 restitutions sur 10 au lieu de 1 | ✅ livré, mesuré |
| Code à hachis (HEDGES) | chaque brin `hedges` : la base est désignée par un hachis de la position, des derniers bits du message (de 8 à 48, selon le rendement) et du sel ; recherche best-first | les insertions et les suppressions comme les substitutions : 1,5 % des trois sortes réunies par base à ¾, 4,5 % à ½, 6 % à ⅓, 15 % à ¼, à couverture 10 | ✅ livré, mesuré |
| Vote par brin | entre les lectures d'un brin `hedges` | une lecture qui rend un autre message que les autres : elle est écartée, à égalité le brin aussi | ✅ livré, mesuré |
| Reed-Solomon externe | chaque bloc de 255 brins `hedges` (32 brins de parité), en diagonale | 32 brins manquants, ou 16 brins faux | ✅ livré, mesuré |

Les mesures de tolérance (plus de deux mille sept cents tirages de simulation, erreurs réelles injectées)
sont dans [les schémas d'encodage](docs/encoding_schemes.md#tolérance-aux-erreurs) — avec celles
des erreurs en salves, moins redoutables que les erreurs dispersées.

### Traitement des données

- **Compression** : zstd s'il rapporte, sinon aucune (LZ4 au choix) ; le choix est inscrit dans
  l'archive, le décodeur n'a rien à deviner
- **Code de Huffman ternaire** (`goldman`) : construit à partir des octets du fichier et porté par
  l'archive ; sans autre compression, il met 1,32 Mio de texte dans 141 000 brins
- **Manifeste** : 12 à 64 oligos porteurs des paramètres, votés à la majorité entre lectures
- **Criblage biochimique** : chaque oligo produit respecte les contraintes, sans exception
- **Marge certifiée** : l'encodeur tient un décodeur à jour et ne livre l'archive que si elle reste
  décodable après des pertes aléatoires, sur 256 tirages indépendants

### Format et conformité

- **FASTA et FASTQ** : lecture et écriture des deux formats, reconnus à leur premier caractère ;
  séquences repliées à 60 nt en FASTA, qualités Phred+33 contrôlées par leur longueur en FASTQ.
  Les en-têtes sont informatifs, le décodage ne les lit pas
- **Illumina** : molécules au format Nextera, index du kit (`N501`–`N508`, `N701`–`N712`) ou
  séquences libres, multiplexage d'un pool et démultiplexage à une erreur par index, dans les
  deux sens de lecture ; lectures appariées coupées des adaptateurs —
  [Bibliothèques Illumina](docs/LIBRARY.md)
- **Contraintes** : bornes de GC et homopolymère configurables par encodage ; des contraintes
  irréalisables sont refusées d'emblée, avec le calcul qui le justifie
- **Format autosuffisant** : décrit précisément dans [docs/FORMAT.md](docs/FORMAT.md)

### Simulation

- **Canal** : substitutions, insertions et suppressions de séquençage ; erreurs de synthèse,
  portées par toutes les lectures d'un oligo ; perte d'oligos ; couverture de Poisson
- **Salves** : un canal à deux états où les erreurs se groupent, comme sur un séquençage réel
- **Reproductible** : générateur déterministe à graines explicites ; un tirage rejoué donne le
  même résultat
- **Bilan** : récupérations, échecs annoncés, fichiers faux — `adn simulate` les compte
- **Séquenceur** : lectures appariées, lectures d'index, erreurs croissantes le long de la
  lecture avec leurs qualités, sauts d'index — branché sur le démultiplexage et le décodeur

### CLI et serveur web

- **CLI** (`adn`) : `encode`, `decode`, `verify`, `simulate`, `visualize` (table, JSON ou HTML),
  `library` et `demux`, sortie `--json` partout
- **Web** (`adn-web`) : dépôt de fichier par glisser-déposer, tâches d'encodage et de décodage,
  téléchargement des résultats. **Pas d'authentification** : réservé à un usage local

---

## 🏗️ Architecture

### Structure du workspace

```
DNAproof/
├── crates/
│   ├── core/         # format v2, six schémas (fountain, strands, goldman, grass, ldpc,
│   │                 # hedges), Reed-Solomon sur GF(256), GF(47) et GF(47²), code
│   │                 # convolutif, LDPC, code à hachis, enveloppe, FASTA et FASTQ,
│   │                 # bibliothèques Illumina
│   ├── cli/          # ligne de commande (clap)
│   ├── web/          # serveur local (actix-web + tera), tâches, téléchargements
│   ├── storage/      # miroir des archives : SQLite ou PostgreSQL (sqlx)
│   └── simulation/   # canal d'erreurs et séquenceur Illumina
├── docs/             # FORMAT.md, schémas, bibliothèques Illumina, performances, API web
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
│ Manifeste         │ 12 à 64 oligos « ADNF » (« HEDG » pour hedges) : schéma, compression,
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

Les schémas de la littérature ont chacun leur chemin, à partir de la même enveloppe :

- `goldman` écrit les octets en trits (code de Huffman ternaire), puis en bases sans répétition ;
  il relit l'ADN par fenêtres de 100 nt décalées de 25 et en fait un brin indexé de 117 nt par
  fenêtre.
- `grass` écrit les octets en symboles de GF(47), les protège par un Reed-Solomon sur GF(47)
  brin par brin, puis par un Reed-Solomon sur GF(47²) bloc par bloc de 713 brins, et pose chaque
  symbole en trois bases.
- `ldpc` passe chaque brin dans un code convolutif, entrelace les bits du bloc, et les protège
  par un LDPC ; au décodage, chaque bit pèse son rapport de vraisemblance.
- `hedges` écrit chaque brin à partir d'un hachis de la position, des bits précédents et de
  l'index du brin, terminé par une queue de zéros ; un Reed-Solomon en diagonale protège les
  brins bloc par bloc, et au décodage une recherche best-first resynchronise chaque lecture —
  substitutions, insertions et suppressions.

### Pipeline de décodage (en sens inverse)

```
Lectures FASTA → normalisation (majuscules, blancs) → manifeste (vote entre lectures)
→ corps selon le schéma (épluchage LT ; code interne puis externe ; vote entre fenêtres recouvrantes ;
  treillis à sorties souples puis LDPC ; recherche best-first, vote par brin puis code externe)
→ contrôle CRC-32 → décompression → contrôle SHA-256 → fichier original
```

Si le corps ne passe pas le contrôle, DNA Fountain rejoue le décodage sans les lectures que la
parité a « corrigées » : elles peuvent l'avoir été à tort. Si l'empreinte finale ne correspond
pas, rien n'est écrit.

### Des archives au séquenceur : les bibliothèques Illumina

```
Archives ──adn library──▶ pool : une molécule Nextera par oligo
   │                        P5 | index i5 | amorce | ME | insert | MEᶜ | amorce | index i7 | P7ᶜ
   │
   └── adn simulate --library ──▶ lectures 1 et 2, index i7 et i5, sauts d'index
                                      │
lectures FASTQ ──── adn demux ────────▶ par échantillon : une erreur tolérée par index, dans les
   │                                     deux sens ; adaptateurs coupés, lecture 2 remise dans
   │                                     le sens de l'insert
   ▼
adn decode ──▶ le fichier d'origine, à l'identique
```

Plusieurs archives partagent une cellule de séquençage, et chacune garde ses propres codes
correcteurs : une lecture rangée dans le mauvais échantillon, par un saut d'index, est écartée
par les siens. Le détail : [Bibliothèques Illumina](docs/LIBRARY.md).

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
| `-a, --algorithm <fountain\|strands\|goldman\|grass\|ldpc\|hedges>` | schéma d'encodage (`fountain` par défaut) |
| `-r, --redundancy <r>` | marge de perte de DNA Fountain (`1.1` par défaut) |
| `--rs-parity <PARITÉ>` | parité du schéma : octets Reed-Solomon par oligo pour DNA Fountain (`2`, jusqu'à 32), symboles par brin pour `grass` (`14`, nombre pair de 2 à 30), brins de parité pour cent pour `ldpc` (`20`, de 5 à 100) |
| `--file-id <0-8>` | identifiant de fichier pour `goldman` : sépare plusieurs archives mélangées dans un même tube |
| `--code-rate <1/4\|1/3\|1/2\|3/4>` | rendement du code de HEDGES (`hedges`) : moins il porte, plus il corrige (`1/2` par défaut) |
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
substitutions, insertions, suppressions, couverture — dispersées, ou en salves avec
`--burst-rate`, `--burst-length` et `--burst-error`) et compte les récupérations sur N tirages ;
`adn visualize` donne les statistiques d'un ensemble de séquences (table, JSON ou HTML).

### Bibliothèques Illumina

```console
$ adn library -o pool.fasta --sample photo.jpg.adn.fasta=N701+N501 \
    --sample chanson.mp3.adn.fasta=N702+N502
Bibliothèque de 2 échantillons écrit dans pool.fasta : 4 003 molécules

Échantillon              Index                               Oligos Insert          Molécule
chanson.mp3.adn          i7 N702 CGTACTAG  i5 N502 CTCTCTAT    2 745 117 à 200 nt    253 à 336 nt
photo.jpg.adn            i7 N701 TAAGGCGA  i5 N501 TAGATCGC    1 258 152 nt          288 nt
```

Chaque archive devient un échantillon du pool, reconnaissable à ses index. Un index est un nom
du kit Nextera ou une séquence ; la bibliothèque refuse deux index qu'une erreur pourrait
confondre. Le séquenceur simulé lit ensuite le pool, et chaque tirage va jusqu'au décodage :

```console
$ adn simulate -i pool.fasta --library --coverage 10 --index-hopping 0.02 -n 3 --reads lectures
Simulation d'un séquenceur sur pool.fasta : 3 tirages, 2 échantillons, 4 003 molécules
Lectures de 300 bases, index de 8 ; couverture 10 ; séquençage 0.10 % / 0.00 % / 0.00 %, index 0.10 % ; …

Tirage  Échantillon                Paires   Fausses  Résultat
1       chanson.mp3.adn             27274       195  récupéré
        photo.jpg.adn               12478       165  récupéré
        sans réponse : 0 ambiguës, 4 inconnues ; 830 sauts d'index
…
Récupérations : 3 sur 3 tirages, tous échantillons restitués
Fichiers faux : 0 : un échec est toujours annoncé
```

Deux pour cent des paires portent les index d'une autre molécule : elles sont rangées dans un
autre échantillon, comptées comme fausses, et les deux fichiers sont pourtant restitués à
l'identique. Sur les lectures d'un vrai séquenceur, `adn demux` sépare les échantillons et coupe
les adaptateurs ; chaque fichier se décode ensuite par `adn decode` :

```console
$ adn demux --r1 lectures/R1.fastq --r2 lectures/R2.fastq --i7 lectures/I1.fastq \
    --i5 lectures/I2.fastq --library pool.fasta --output demultiplexe --truth
Démultiplexage de 39 756 paires de lectures : 2 échantillons, sortie dans demultiplexe

Échantillon                  Paires     Lectures  Fichier
chanson.mp3.adn              27 274       54 548  demultiplexe\chanson.mp3.adn.fasta
photo.jpg.adn                12 478       24 956  demultiplexe\photo.jpg.adn.fasta
Sans réponse (ambiguës)           0
Sans réponse (inconnues)           4
Origine des lectures         39 392 justes ; 360 fausses
    chanson.mp3.adn → photo.jpg.adn : 165 lectures
    photo.jpg.adn → chanson.mp3.adn : 195 lectures
```

Les échantillons viennent du pool (`--library`) ou de leurs index (`--sample <i7>[+<i5>][=<nom>]`).
Avec `--truth`, les lectures simulées disent d'où ils viennent, et les attributions fausses se
comptent : le format, le séquenceur et les mesures sont dans [Bibliothèques
Illumina](docs/LIBRARY.md).

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

### Goldman et al., 2013

- **Brin** : une base de repère, 100 nt de message, un index de 15 trits (identifiant de fichier,
  adresse, trit de parité), une seconde base de repère = 117 nt ; jamais deux bases identiques
- **Code ternaire** : Huffman construit sur les octets du fichier, porté par l'archive ; à défaut,
  code fixe de 5 ou 6 trits
- **Fenêtres** : l'ADN est relu par fenêtres de 100 nt décalées de 25 ; chaque base du message
  apparaît dans quatre brins, une fenêtre sur deux est prise en sens inverse
- **Décodage** : une voix par brin qui couvre une base ; une répétition de bases, impossible dans
  un brin conçu, écarte la lecture du vote à cet endroit
- **Limite** : l'adresse tient sur 12 trits, d'où le corps maximal de 2,6 Mo ; pas de code
  correcteur, donc pas de GC garanti par brin, et une couverture nécessaire

### Grass et al., 2015

- **Brin** : index de 4 symboles + nonce + charge utile de 20 symboles + parité interne de
  14 symboles = 39 symboles de GF(47), soit 117 nt (un trimère par symbole)
- **Trimères** : 47 des 64 trimères possibles, choisis pour qu'aucune suite de plus de trois
  bases identiques n'apparaisse ; un trimère hors du jeu est un effacement
- **Code interne** : Reed-Solomon sur GF(47) par brin, corrige 7 symboles erronés par lecture
- **Code externe** : Reed-Solomon sur GF(47²), 92 brins de parité par bloc de 713 (le bloc de
  l'article), colonne par colonne
- **Limite** : l'index tient sur 47⁴, d'où le corps maximal de 55 Mo

### Code convolutif et LDPC

- **Brin** : sel de 20 bits, index de 24 bits, charge utile de 13 octets, queue de six zéros :
  148 bits passent dans le code convolutif (171, 133)₈ — longueur de contrainte 7, rendement ½,
  distance libre 10 —, soit 154 nt
- **Lecture** : chaque base pèse ±4 par bit ; l'algorithme log-MAP rend, par bit d'information,
  un rapport de vraisemblance borné à ±4 ; les lectures d'un même brin s'additionnent
- **Entrelacement** : dans le mot de code d'un bloc, les bits de même rang passent consécutivement
- **LDPC** : par bloc de 819 brins de données, quatre contrôles par bit d'information, sans
  cycle de quatre, matrice reconstruite d'une graine ; min-sum normalisé, 60 allers-retours au
  plus ; 20 % de brins de parité par défaut
- **Limite** : l'index tient sur 24 bits, d'où le corps maximal d'environ 173 Mio ; les
  lectures qui changent de longueur sont écartées, comme ailleurs

### HEDGES (Press et al., 2020)

- **Brin** : l'index de 24 bits sert de sel, puis la charge utile (13 à 32 octets selon le
  rendement) et une queue de 16 à 20 zéros ; chaque base porte zéro, un ou deux bits du message,
  suivant le motif du rendement — 200 nt à ¾ et à ½, 300 à ⅓ et à ¼
- **Base émise** : `A[(K + v) mod |A|]` — `v` porte les bits du message, `A` est l'ensemble des
  bases admises, et `K` un hachis de la position, des derniers bits du message et du sel. Le
  message ne se lit pas dans l'ADN : il se déduit du hachis
- **Lecture** : une recherche best-first sur l'arbre des hypothèses ; une base qui correspond
  rapporte, une erreur coûte ; une chaîne qui suppose trop d'erreurs est abandonnée, et un
  message rival presque à égalité fait échouer la lecture
- **Contraintes** : dans le code, sans criblage — jamais plus de trois bases identiques, et la
  part de GC du brin tenue entre 45 et 55 %
- **Code externe** : Reed-Solomon (255, 223) en diagonale entre les brins ; 32 brins de parité
  par bloc, les lectures d'un brin votent
- **Limite** : l'index tient sur 24 bits, d'où le corps maximal d'environ 182 Mio à ¼ et 448 Mio
  à ¾ ; une lecture qui résiste coûte une recherche entière : 4,3 ms, 310 fois une
  lecture intacte

### Bibliothèques Illumina

- **Molécule** : format Nextera — P5, index i5, site de la lecture 1, extrémité mosaïque, insert,
  extrémité mosaïque complément inversée, site de la lecture 2, index i7, P7 complément inversé ;
  136 nt d'adaptateurs avec des index de 8 bases
- **Index** : ceux du kit Nextera (`N501`–`N508`, `N701`–`N712`) ou toute séquence de 4 à 12
  bases ; deux index d'une même position à trois bases l'un de l'autre au moins, jamais
  compléments inversés l'un de l'autre
- **Lectures** : la lecture 1 part du début de l'insert, la lecture 2 de son extrémité mosaïque
  sur l'autre brin, les index avec P7 et P5 ; la longueur de lecture doit atteindre le plus long
  insert, manifestes compris
- **Démultiplexage** : une erreur par index, dans les deux sens de lecture ; à égalité, la paire
  reste sans réponse ; adaptateurs coupés, lecture 2 remise dans le sens de l'insert

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
| Cœur, unitaires | 195 | enveloppe, manifeste, gouttes, brins, fenêtres goldman, brins grass, brins ldpc, Reed-Solomon sur GF(256), GF(47) et GF(47²), code convolutif et LDPC, code à hachis et schéma hedges, contraintes, générateur, FASTA et FASTQ, bibliothèques Illumina |
| Aller-retour format | 10 | `encode → FASTA → decode == original` pour les six schémas, en-têtes renommés ou perdus, CRLF et minuscules, contraintes strictes |
| Robustesse | 8 | substitutions corrigées, marge honorée puis refusée au-delà, brins manquants réparés, manifestes abîmés, altérations au hasard : jamais un fichier faux |
| Propriétés (proptest) | 2 | aller-retour sur entrées au hasard (schéma × type × taille × mélange) et à contraintes aléatoires |
| Simulation, bout en bout | 12 | encodage → canal d'erreurs → décodage, 12 scénarios de canal |
| Simulation, unitaires | 23 | modèle du canal, statistiques, erreurs en salves, indépendance des tirages ; séquenceur : lectures appariées, index, sauts d'index, qualités |
| Stockage | 13 | SQLite et PostgreSQL : lots, transactions, reprise après redémarrage |
| CLI | 28 | les sept sous-commandes exécutées comme par un utilisateur, dont deux archives goldman mélangées, un pool de deux échantillons séquencé, démultiplexé et décodé, le schéma ldpc aux erreurs et aux pertes, et le schéma hedges aux insertions, aux suppressions et aux pertes |
| Serveur web | 38 | pages, API, refus, en-têtes, reprise, hôtes |
| Exemple du README | 1 | le code montré plus haut, tel quel |
| Exemple de la bibliothèque | 2 | les doc-tests de `adn-core` : l'archive, et une paire de lectures rangée |
| **Total** | **332** | **0 échec** (+ 5 ignorés : 4 tests d'échelle, 1 PostgreSQL sans serveur) |

### Propriétés clés

- **Tous les aller-retours sont stricts** : `assert_eq!(original, decoded)` — aucun raccourci du
  genre « vérifier que ce n'est pas vide »
- **Les contraintes sont vérifiées oligo par oligo**, pas échantillonnées
- **Un fichier faux n'a jamais été produit** : sur les plus de deux mille sept cents tirages des
  matrices de tolérance et les tests d'altération au hasard, chaque contenu reconstitué était
  exact, et chaque archive trop abîmée a été refusée avec un message
- **Le canal de simulation est branché sur le décodeur** : les tests de bout en bout vérifient la
  récupération réelle, pas seulement un aller-retour sans bruit

---

## 📈 Performances mesurées

`cargo bench -p adn-core`. Construction `--release`, sur un AMD Ryzen AI Max+ 395 (32 threads).
Fichiers incompressibles — le détail, les grandes tailles et la mémoire dans
[Performances](docs/PERFORMANCE.md) :

| | 64 Kio | 256 Kio | 1 Mio |
|---|---|---|---|
| Encodage `fountain` (marge 1,1) | 40 ms | 80 ms | 215 ms |
| Encodage `strands` | 39 ms | 39 ms | 45 ms |
| Encodage `goldman` | 31 ms | 47 ms | 114 ms |
| Encodage `grass` | 68 ms | 103 ms | 267 ms |
| Encodage `ldpc` | 80 ms | 113 ms | 175 ms |
| Encodage `hedges` | 4,4 ms | 14,1 ms | 51,9 ms |
| Décodage `fountain`, archive intacte | 4,3 ms | 13 ms | 65 ms |
| Décodage `strands`, archive intacte | 3,0 ms | 10 ms | 31 ms |
| Décodage `goldman`, archive intacte | 19 ms | 76 ms | 292 ms |
| Décodage `grass`, archive intacte | 11 ms | 23 ms | 75 ms |
| Décodage `ldpc`, archive intacte | 69 ms | 121 ms | 289 ms |
| Décodage `fountain`, 0,1 % de substitution, 3 lectures par oligo | 7,4 ms | 25 ms | 113 ms |
| Décodage `strands`, 0,5 % de substitution | 5,3 ms | 14 ms | 46 ms |
| Décodage `goldman`, 0,1 % de substitution, 3 lectures par brin | 45 ms | 179 ms | 693 ms |
| Décodage `grass`, 0,5 % de substitution | 14 ms | 33 ms | 113 ms |
| Décodage `ldpc`, 1 % de substitution, 3 lectures par brin | 283 ms | 934 ms | 3,55 s |
| Décodage `hedges`, archive intacte | 87 ms | 140 ms | 328 ms |
| Décodage `hedges`, 1 % de substitution, 3 lectures par brin | 333 ms | 554 ms | 2,12 s |

Le schéma `goldman` produit quatre fois plus de brins que les autres pour un même fichier, et
ses lectures y vont de trois : c'est le nombre de lectures qui fait son coût. Le schéma `ldpc`
est le plus lent à décoder : chaque lecture fautive passe sur un treillis de 64 états, et son
décodage compte en secondes dès le mébioctet — intacte, l'archive se relit dix fois plus vite.
Le schéma `hedges` paie ses lectures une par une : une recherche de 13,9 µs si elle est intacte,
63,7 µs à 1 % d'erreur de chaque sorte, 143 µs au rendement ¾ — et 4,3 ms si elle résiste, 310
fois plus : une lecture d'une autre archive, rangée dans la mauvaise par un saut d'index, ou
plus abîmée que ce que son rendement corrige.

En grand : 32 Mio s'encodent en 6,9 s avec `fountain` (1,6 Go de mémoire), en 0,9 s avec
`strands` (0,4 Go). Sur un fichier texte, la compression compte davantage que le schéma : 1,32 Mio
de texte tient en 9 982 oligos une fois compressé par zstd, contre 39 000 pour un Mio d'octets au
hasard.

La chaîne Illumina ajoute peu : `adn demux` range environ 215 000 paires de lectures par
seconde, et `adn library` assemble 24 673 molécules en 40 ms —
[Bibliothèques Illumina](docs/LIBRARY.md#mesures).

---

## ⚠️ Limites connues

- **Pas de vraie synthèse** : logiciel uniquement ; aucune interface avec du matériel ou des
  prestataires de synthèse et de séquençage
- **Pas d'authentification dans le serveur web** : usage local ; ajouter une authentification
  devant (proxy) pour tout autre usage
- **Erreurs de lecture** : avec sa parité par défaut (deux octets), DNA Fountain ne corrige qu'une
  erreur par oligo : dès 0,1 % de substitution par nucléotide, il faut plusieurs lectures par
  oligo, et une vingtaine à 1 %. `strands` corrige deux octets par brin, et jusqu'à 32 brins
  manquants sur 255 — 5 % en pratique sur un gros fichier. `grass` va le plus loin : 5 % de
  substitution, à couverture 3
- **Le schéma `goldman` n'a pas de code correcteur** : chaque base doit être lue dans l'un de ses
  quatre brins, et une erreur dans l'index d'un brin le fait perdre. Il demande une couverture de
  5, et ne garantit pas la teneur en GC de ses brins
- **Insertions et suppressions** : hors `hedges`, une lecture qui change de longueur est
  écartée. À 0,5 % de chaque sorte, `fountain` ne s'en tire qu'à marge large, et pas à tous
  les coups ; `strands`, `goldman` et `ldpc` jamais ; `grass` seulement à forte couverture
- **Erreurs de synthèse** : portées par toutes les lectures d'un oligo, elles ne se compensent
  pas : il faut plus de parité par oligo (`--rs-parity 8`). `grass` les traverse jusqu'à 1 % ;
  `goldman` les perd dès qu'elles touchent un index. `ldpc` les lit comme des erreurs de
  lecture : 5 % à couverture 10, une fois sur deux. `hedges` les corrige à sa place
- **Le schéma `ldpc`** demande 2,5 lectures par brin en moyenne au moins, perd les lectures dont
  la longueur a changé, et décode lentement. Son LDPC est régulier, sans distribution de degrés
  optimisée : pris seul, il corrige 2 % d'erreurs à rendement 0,8, loin de la capacité du canal
  (10 %) ; il faut une parité de 50 % pour passer de 10 à 22 % de perte
- **Petits fichiers** : DNA Fountain leur donne beaucoup de redondance (141 oligos pour 75
  octets), et `grass` un bloc entier de parité (103 brins) ; `strands` est le plus économe
- **Taille** : `goldman` s'arrête à 2,6 Mo, `grass` à 55 Mo, `hedges` à 182 Mio (rendement ¼)
  et à 448 Mio (¾), `ldpc` à 173 Mio, `strands` à 420 Mio ; au-delà, seul `fountain` convient,
  et son encodage demande environ cinquante fois la taille du fichier en mémoire
- **Le serveur web règle peu de choses** : la marge de DNA Fountain et le rendement de `hedges`,
  mais ni la parité (`--rs-parity`), ni les contraintes biochimiques — pour un canal difficile,
  passer par la ligne de commande
- **Bibliothèques Illumina** : les lectures partielles ne s'assemblent pas — la longueur de
  lecture doit atteindre le plus long insert du pool, et 19 bases de plus pour que la lecture 2
  serve ; seul le format Nextera est décrit, et la qualité des lectures n'entre pas dans le
  décodage
- **Le schéma `hedges` a ses propres limites**. La perte s'arrête où la parité du code externe
  le dit : 12,5 % d'un bloc, 7 % en pratique. Ses brins font 200 ou 300 nt : la longueur de
  lecture doit les couvrir. Une lecture qui résiste coûte une recherche entière : 4,3 ms,
  310 fois une lecture intacte — c'est le prix d'une lecture d'une autre archive, et de
  toute lecture plus abîmée que ce que son rendement corrige. Et le code ne garantit que les
  contraintes par défaut : des bornes de GC plus étroites, ou deux bases identiques au plus,
  lui sont refusées
- **Au-delà de ce qu'un code corrige, un fichier faux reste possible en théorie** : à ¼,
  à 10 % d'erreurs, 1,9 lecture sur mille rend un message faux, 3,8 à 12 %. Le vote, le code externe et l'empreinte du fichier l'arrêtent — aucun
  fichier faux n'a été rendu

---

## 🚀 Feuille de route

### État actuel : v0.2.0

- ✅ Format v2 autosuffisant : manifeste embarqué, empreinte SHA-256 vérifiée à chaque décodage
- ✅ Six schémas (`fountain`, `strands`, `goldman`, `grass`, `ldpc`, `hedges`) à aller-retour
  strict, dont les deux schémas pionniers de la littérature, suivis article par article
- ✅ Marge de perte certifiée à l'encodage ; décodeurs durcis (vote par goutte, code externe par
  bloc)
- ✅ Simulation réaliste (synthèse, séquençage, perte, couverture) branchée sur le décodeur
- ✅ Matrices de tolérance mesurées : plus de deux mille sept cents tirages indépendants,
  aucun fichier faux
- ✅ Codes convolutif et LDPC, entrelacement, erreurs en salves : `ldpc` traverse 5 % de
  substitution, 5 % d'erreurs de synthèse une fois sur deux, et 20 % d'erreurs en salves
- ✅ HEDGES : le seul schéma qui corrige les insertions et les suppressions — 4,5 % d'erreurs
  des trois sortes réunies par base au rendement ½, 15 % au rendement ¼, 18 % à couverture
  20 —, manifeste compris
- ✅ Bibliothèques Illumina : pool multiplexé au format Nextera, séquenceur simulé,
  démultiplexage à une erreur par index — à 30 % de sauts d'index, chaque échantillon est
  restitué à l'identique
- ✅ CLI complète (`encode`, `decode`, `verify`, `simulate`, `visualize`, `--json`)
- ✅ Serveur web avec suivi de tâches, reprise après redémarrage, base SQLite ou PostgreSQL
- ✅ CI au vert : format, Clippy, tests (Ubuntu et Windows), MSRV 1.94, PostgreSQL, audit des
  dépendances, image Docker
- ✅ Documentation refaite sur des mesures : [format](docs/FORMAT.md),
  [schémas](docs/encoding_schemes.md), [performances](docs/PERFORMANCE.md),
  [production](PRODUCTION_GUIDE.md), [API web](docs/api_reference.md)

### Le programme

La direction du projet : couvrir, pour de vrai, ce que la littérature a proposé de meilleur.

1. ✅ **Les schémas publiés** — Goldman 2013 (code ternaire) et Grass 2015 (Reed-Solomon sur
   GF(47)), suivis article par article, avec mesure de tolérance.
2. ✅ **Le format des bibliothèques Illumina** — adaptateurs P5/P7, index i5/i7, multiplexage et
   démultiplexage tolérants aux erreurs, avec un séquenceur simulé pour les mesurer.
3. ✅ **Les familles de codes correcteurs** — code convolutif et décodage de Viterbi, LDPC à
   propagation de croyances capable de corriger, entrelacement contre les salves d'erreurs.
4. ✅ **Les insertions et suppressions** — HEDGES (Press et al., 2020), code à hachis par
   brin décodé par une recherche, mesuré avec sa matrice de tolérance.

Ensuite :

- [ ] Parité réglable depuis le serveur web (aujourd'hui : marge de perte et rendement de
  `hedges`)
- [ ] Les codes apparentés à HEDGES — codes VT, codes à filigrane — et l'accélération de sa
  recherche (GPU)
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
- [Bibliothèques Illumina : molécules, index, lectures, démultiplexage](docs/LIBRARY.md)
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
  [doi:10.1002/anie.201411378](https://onlinelibrary.wiley.com/doi/10.1002/anie.201411378)
- Goldman N. et al. — *Towards practical, high-capacity, low-maintenance information storage in
  synthesised DNA*, Nature 494:77–80, 2013.
  [doi:10.1038/nature11875](https://www.nature.com/articles/nature11875)
- Luby M. — *LT codes*, FOCS 2002.
- Viterbi A. — *Error bounds for convolutional codes and an asymptotically optimum decoding
  algorithm*, IEEE Transactions on Information Theory 13(2):260–269, 1967.
- Bahl L. R., Cocke J., Jelinek F., Raviv J. — *Optimal decoding of linear codes for minimizing
  symbol error rate*, IEEE Transactions on Information Theory 20(2):284–287, 1974.
- Gallager R. G. — *Low-density parity-check codes*, MIT Press, 1963.
- Press W. H., Hawkins J. A., Jones S. K. Jr., Schaub J. M., Finkelstein I. J. — *HEDGES
  error-correcting code for DNA storage corrects indels and allows sequence constraints*,
  Proceedings of the National Academy of Sciences 117:18489–18496, 2020.
  [doi:10.1073/pnas.2004821117](https://www.pnas.org/doi/10.1073/pnas.2004821117)
- Zigangirov K. S. — *Some sequential decoding procedures using a criterion*, Problems of
  Information Transmission 2(4):13–25, 1966 ; Jelinek F. — *A fast sequential decoding
  algorithm using a stack*, IBM Journal of Research and Development 13(6):675–685, 1969 :
  le décodage séquentiel à pile, dont la recherche de HEDGES procède

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
