# Bibliothèques Illumina

Une archive est un jeu d'oligos. Pour la faire lire par un séquenceur, chaque oligo devient
l'**insert** d'une molécule, entourée d'adaptateurs et d'**index**. Plusieurs archives peuvent
ainsi être mélangées dans une même cellule — un **pool** — puis séparées à la lecture : c'est le
**multiplexage**. Ce document décrit la brique qui s'en charge, d'Illumina au décodage.

```
archives (.adn.fasta) ──adn library──▶ pool.fasta (une molécule par oligo)
                                          │
                             adn simulate --library
                             (modèle de séquenceur)
                                          │  R1, R2, I1, I2 en FASTQ
                                          ▼
 Lectures d'un séquenceur ──────adn demux──────▶ <échantillon>.fasta (lectures dans le sens de l'insert)
                                                     │
                                                 adn decode
                                                     ▼
                                              le fichier d'origine
```

Tout ce qui suit a été mesuré ; les chiffres sont dans [Mesures](#mesures).

## La molécule

Le format est celui des kits Nextera, d'après le document *Illumina Adapter Sequences*
(1000000002694 v01) :

| P5 | index i5 | site de la lecture 1 | ME | **insert** | MEᶜ | site de la lecture 2 | index i7 | P7ᶜ |
|---|---|---|---|---|---|---|---|---|
| 29 nt | 4 à 12, 8 pour le kit | 14 nt | 19 nt | l'oligo | 19 nt | 15 nt | 4 à 12, 8 pour le kit | 24 nt |

| Élément | Séquence | Rôle |
|---------|----------|------|
| P5 | `AATGATACGGCGACCACCGAGATCTACAC` | accroche à la cellule ; amorce de la lecture d'index i5 |
| P7 | `CAAGCAGAAGACGGCATACGAGAT` | accroche à l'autre bout ; amorce de la lecture d'index i7 |
| Site de la lecture 1 | `TCGTCGGCAGCGTC` | l'amorce s'y fixe : la lecture 1 part du début de l'insert |
| ME | `AGATGTGTATAAGAGACAG` | extrémité mosaïque de la transposase |
| MEᶜ | `CTGTCTCTTATACACATCT` | ME complément inversée, après l'insert |
| Site de la lecture 2 | `CCGAGCCCACGAGAC` | l'amorce s'y fixe : la lecture 2 part de la fin de l'insert, sur l'autre brin |

Avec des index de 8 bases, les adaptateurs occupent 136 nt : un oligo de 152 nt donne une
molécule de 288 nt, un de 117 nt une molécule de 253 nt.

Les index du kit Nextera sont donnés dans le sens de la molécule : pour i7, les bases de la
feuille d'échantillon (`N701` = `TAAGGCGA`) ; pour i5, celles de l'adaptateur (`N501` =
`TAGATCGC`). Le document d'Illumina demande, pour MiniSeq, NextSeq et HiSeq 3000/4000, le
complément inversé de l'index i5 dans la feuille d'échantillon : le sens d'un index lu dépend
donc de la plateforme.

## Les lectures

Le séquenceur lit chaque molécule plusieurs fois. De chaque lecture, il tire quatre séquences :

| Lecture | D'où elle part | Ce qu'elle contient |
|---|---|---|
| 1 (R1) | la première base de l'insert | l'insert, puis MEᶜ, le site de la lecture 2, i7, P7ᶜ |
| 2 (R2) | ME, sur l'autre brin | ME, l'insert à l'envers, puis ce qui le précède dans la molécule |
| Index 1 (I1) | P7 | i7 complément inversé |
| Index 2 (I2) | P5 | i5, dans le sens de la molécule |

Une lecture s'arrête au bout de la molécule.

**Longueur de lecture.** Pour qu'une archive se décode, chaque oligo doit être lu en entier :
la longueur de lecture doit atteindre le plus long insert du pool, manifestes compris. Avec les
réglages par défaut : 152 nt pour `fountain` et les manifestes de `goldman`, 160 pour `strands`,
154 pour `ldpc`, 200 pour les manifestes de `grass`, 200 ou 300 pour `hedges` — 300 à ⅓ et à ¼,
200 à ½ et à ¾, manifestes compris. Une lecture de 300 bases, courant sur les machines de la
gamme, couvre tout. En dessous, les lectures 1 partielles ne s'assemblent pas : elles sont
écrites telles quelles, et le décodeur en rejette la longueur — seul `hedges` complète une
lecture arrêtée dans la queue de zéros de son brin.

La lecture 2 ne sert que si elle couvre l'insert en entier : elle part de l'extrémité
mosaïque, 19 bases plus loin, et il lui faut 19 bases de plus que la lecture 1 pour arriver au
début de l'insert. À 300 bases, c'est le cas d'un insert de 200 nt, pas d'un insert de 300 nt —
sa lecture 2 donne les 281 dernières bases du brin. Une telle lecture est écartée : `adn demux`
ne l'écrit pas, et la compte parmi les « lectures 2 partielles ». Aucun schéma ne lit la fin
d'un brin sans son début ; celui de `hedges` la cherchait en vain, jusqu'au bout de son budget.

**La coupe des adaptateurs.** `adn demux` rend chaque lecture dans le sens de l'insert :

- la lecture 1 est coupée à la première occurrence du début de MEᶜ — dix bases exactes, ou les
  dix-neuf à une erreur près ;
- la lecture 2 est complément inversée : ses dix-neuf dernières bases sont MEᶜ (ou sa fin, si la
  lecture s'est arrêtée dedans), et si elle a traversé l'insert, ce qu'elle a lu après lui se
  retrouve avant lui, terminé par ME entière, à une erreur près ;
- une lecture 2 qui n'a pas rejoint le début de l'insert est écartée : l'extrémité mosaïque qui
  le précède n'y est pas reconnaissable, et son extrait est plus court que celui de la lecture 1.
  Aussi long ou plus long, on ne peut rien conclure, et elle est gardée.

Un dépassement de moins de dix bases n'est pas reconnaissable : la lecture reste telle quelle,
et le décodeur en rejette la longueur.

## Le démultiplexage

Une paire de lectures est rangée d'après ses lectures d'index :

- chaque index lu doit être à **une erreur au plus** d'un index d'échantillon — une base
  remplacée, ou manquante en bout de lecture ;
- chaque index est comparé **dans les deux sens** de lecture ;
- l'échantillon qui totalise le moins d'erreurs l'emporte ; à égalité, la paire est déclarée
  **ambiguë** : elle n'est rangée nulle part.

Pour que l'erreur ne puisse pas confondre deux échantillons, une bibliothèque est vérifiée à la
construction :

- deux index d'une même position diffèrent d'au moins **trois bases** ;
- aucun n'est le complément inversé d'un autre ;
- ils ont même longueur à une même position ; les noms d'échantillons sont uniques ; l'index i5
  est présent partout ou nulle part.

Le kit Nextera satisfait la première règle avec au moins quatre bases d'écart. Mais deux de ses
paires n'écartent que de deux bases le complément inversé de l'un de l'index de l'autre :
`N707`/`N712` et `N503`/`N504`. La bibliothèque les admet — une lecture exacte ne se confond
jamais — et une lecture à une erreur des deux, dans le sens qui les rapproche, est déclarée
ambiguë. C'est le seul cas où `adn demux` laisse une paire sans réponse alors que ses index
n'ont qu'une erreur, et l'autre index tranche dès qu'on le lit.

## Le séquenceur simulé

`adn simulate --library` modélise un séquenceur en amont du canal habituel :

- **Couverture** : le nombre de paires de lectures d'une molécule suit une loi de Poisson de
  moyenne `--coverage` ; une molécule absente (`--dropout`) ou sans lecture est perdue.
- **Erreurs** : chaque lecture reçoit ses propres erreurs. Leur probabilité croît du début à la
  fin de la lecture — un sixième de la moyenne au début, presque le double à la fin — et la
  qualité Phred inscrite à chaque base est celle de sa probabilité d'erreur. Les lectures d'index
  ont leur propre taux.
- **Sauts d'index** (`--index-hopping`) : une fraction des paires porte les index d'une autre
  molécule du pool. Le phénomène, observé sur les cellules à motif, range la paire dans un autre
  échantillon.
- **Synthèse** : les erreurs de synthèse frappent l'insert d'une molécule ; toutes ses lectures
  les portent.

Les lectures s'écrivent en FASTQ (`--reads <dossier>` : `R1.fastq`, `R2.fastq`, `I1.fastq`,
`I2.fastq`). Chaque en-tête porte l'échantillon d'origine, `sample=<nom>` : `adn demux --truth`
le confronte au rangement et compte les attributions fausses.

## En ligne de commande

```console
$ adn library -o pool.fasta \
    --sample photo.jpg.adn.fasta=N701+N501 \
    --sample chanson.mp3.adn.fasta=N702+N502
Bibliothèque de 2 échantillons écrit dans pool.fasta : 4 003 molécules

Échantillon              Index                               Oligos Insert          Molécule
chanson.mp3.adn          i7 N702 CGTACTAG  i5 N502 CTCTCTAT    2 745 117 à 200 nt    253 à 336 nt
photo.jpg.adn            i7 N701 TAAGGCGA  i5 N501 TAGATCGC    1 258 152 nt          288 nt
```

Un index est un nom Nextera (`N701`) ou une séquence (`TAAGGCGA`). Sans nom, l'échantillon
prend celui du fichier d'archive ; pour en imposer un : `--sample nom=archive=i7+i5`.

```console
$ adn simulate -i pool.fasta --library --coverage 10 --index-hopping 0.05 -n 3 --reads lectures
```

Chaque tirage passe par le séquenceur, le démultiplexage et le décodage de chaque échantillon.
Le bilan compte les échantillons restitués à l'identique ; un fichier qui ne porte pas
l'empreinte de son archive n'est jamais rendu.

```console
$ adn demux --r1 lectures/R1.fastq --r2 lectures/R2.fastq \
    --i7 lectures/I1.fastq --i5 lectures/I2.fastq \
    --library pool.fasta --output demultiplexe --truth

$ adn demux --r1 R1.fastq --r2 R2.fastq --i7 I1.fastq \
    --sample N701+N501=photo --sample N702=chanson --output demultiplexe
$ adn decode -i demultiplexe/photo.fasta -o photo.jpg
```

Les échantillons viennent du pool (`--library`) ou de leurs index (`--sample`, répétable, avec
un nom facultatif après `=`). Sans lecture d'index i5, le rangement se fait sur le seul index
i7. Les lectures peuvent être en FASTQ ou en FASTA ; elles doivent être appariées, dans le même
ordre dans les quatre fichiers. Chaque échantillon s'écrit dans `<sortie>/<nom>.fasta`.

## En bibliothèque

```rust
use adn_core::library::{place, Placement};
use adn_core::{Index, Library, Sample};

let library = Library::new(vec![
    Sample {
        name: "photo".into(),
        i7: Index::nextera_i7("N701").unwrap(),
        i5: Some(Index::nextera_i5("N501").unwrap()),
    },
    Sample {
        name: "chanson".into(),
        i7: Index::nextera_i7("N702").unwrap(),
        i5: Some(Index::nextera_i5("N502").unwrap()),
    },
])
.unwrap();
// La molécule d'un oligo, prête pour le pool.
let molecule = library.molecule(0, &oligo).unwrap();
// Une paire de lectures, rangée et coupée.
match place(&library, &r1, &r2, &i7, Some(&i5)) {
    Placement::Sample(reads) => { /* reads.sample, reads.r1, reads.r2 */ }
    Placement::Ambiguous => {} // deux échantillons aussi proches
    Placement::Unknown => {}    // aucun à une erreur ou moins
}
```

Le module `adn_core::library` porte les constantes du format, les index du kit Nextera, la
construction et la vérification des molécules, le démultiplexage et la coupe. `adn_simulation`
fournit le séquenceur (`Sequencer`, `SequencerConfig`) et ses en-têtes. La lecture de fichiers
reconnaît le FASTQ comme le FASTA (`adn_core::fasta::read_file`).

## Mesures

Quatre échantillons de 8 Kio d'octets au hasard, un par schéma (`fountain`, `strands`, `goldman`,
`grass`), index `N701`–`N704` et `N501`–`N504` : 3 724 molécules. Lectures de 300 bases,
couverture de 10 (37 240 paires par tirage), cinq tirages par scénario. Construction `--release`.

| Scénario | Tirages entièrement restitués | Attributions fausses | Paires sans réponse |
|---|:---:|---:|---:|
| Référence (0,1 % d'erreur) | 5 sur 5 | 0 | 11 |
| Sauts d'index de 0,1 % | 5 sur 5 | 120 | 11 |
| Sauts d'index de 1 % | 5 sur 5 | 1 216 | 11 |
| Sauts d'index de 5 % | 5 sur 5 | 6 364 | 11 |
| Sauts d'index de 10 % | 5 sur 5 | 12 663 | 11 |
| Sauts d'index de 30 % | 5 sur 5 | 37 867 | 11 |
| Substitutions de 0,5 % | 5 sur 5 | 0 | 11 |
| Substitutions de 1 % | 5 sur 5 | 0 | 11 |
| Substitutions de 2 % | 4 sur 5 | 0 | 11 |
| Erreurs d'index de 1 % | 5 sur 5 | 0 | 979 |
| Erreurs d'index de 5 % | 5 sur 5 | 0 | 20 878 |
| Substitutions de 1 % et sauts d'index de 5 % | 5 sur 5 | 6 364 | 11 |
| Erreurs de synthèse de 1 % | 0 sur 5 | 0 | 11 |
| Perte de 10 % des molécules | 4 sur 5 | 0 | 10 |
| Couverture de 3 | 5 sur 5 | 0 | 1 |
| Couverture de 1 | 0 sur 5 | 0 | 0 |
| Lectures de 200 bases | 5 sur 5 | 0 | 11 |
| Lectures de 150 bases | 0 sur 5 | 0 | 11 |

- **Les lectures étrangères ne corrompent rien.** À 30 % de sauts d'index, un cinquième des
  paires est rangée dans un autre échantillon que le sien, et chaque échantillon est pourtant
  restitué à l'identique : les codes des quatre schémas écartent les lectures d'une autre archive.
- **Aucune fausse attribution sans saut d'index.** Jusqu'à 5 % d'erreur dans les index, une
  paire se range à bon escient ou reste sans réponse : 0,5 % des paires restent sans réponse à
  1 % d'erreur dans les index, 11 % à 5 %.
- **Le reste est celui du canal**
  [hors bibliothèque](encoding_schemes.md#tolérance-aux-erreurs) — la couverture compte les
  paires, donc dix paires font vingt lectures par oligo. À 1 % d'erreur de synthèse, seul
  `grass` passe : `fountain`, `strands` et `goldman` échouent à chacun des cinq tirages. À 10 %
  de perte, le pool n'est plus restitué que quatre fois sur cinq : `goldman` perd un tirage —
  à 2 % de substitution, c'est `fountain`.
- **Durées** (même machine, construction `--release`) : `adn library` assemble quatre archives
  de 64 Kio (24 964 molécules) en 42 ms ; `adn demux` range environ 215 000 paires de lectures
  par seconde (250 084 paires en 1,17 s, 398 921 en 1,86 s) ; `adn simulate --library` tire,
  démultiplexe et décode 250 084 paires en 2,6 s.

Un second pool, cette fois de quatre échantillons de 8 Kio tous au schéma `hedges`, un par
rendement — ¼, ⅓, ½, ¾, dans cet ordre ci-dessous —, index `N701`–`N704` et `N501`–`N504` :
2 197 molécules, soit 21 962 paires par tirage à couverture de 10. Lectures de 300 bases, cinq
tirages par scénario.

| Scénario | Tirages entièrement restitués | Attributions fausses | Paires sans réponse | Échantillons restitués |
|---|:---:|:---:|:---:|---|
| Référence (0,1 % d'erreur), couverture 10 | 5 sur 5 | 0 | 4 | 5/5, 5/5, 5/5, 5/5 |
| Insertions, suppressions et substitutions de 1 %, couverture 10 | 5 sur 5 | 0 | 4 | 5/5, 5/5, 5/5, 5/5 |
| Insertions, suppressions et substitutions de 3 %, couverture 10 | 5 sur 5 | 0 | 4 | 5/5, 5/5, 5/5, 5/5 |
| Insertions, suppressions et substitutions de 3 %, couverture 20 | 5 sur 5 | 0 | 10 | 5/5, 5/5, 5/5, 5/5 |
| Substitutions de 1 %, couverture 10 | 5 sur 5 | 0 | 4 | 5/5, 5/5, 5/5, 5/5 |
| Synthèse : insertions, suppressions et substitutions de 3 %, couverture 10 | 0 sur 5 | 0 | 4 | 5/5, 5/5, 5/5, 0/5 |
| Sauts d'index de 10 %, couverture 10 | 5 sur 5 | 7 911 | 4 | 5/5, 5/5, 5/5, 5/5 |
| Lectures de 200 bases | 0 sur 5 | 0 | 4 | 0/5, 0/5, 5/5, 5/5 |
| Lectures de 150 bases | 0 sur 5 | 0 | 4 | 0/5, 0/5, 0/5, 0/5 |
| Couverture de 3 | 5 sur 5 | 0 | 1 | 5/5, 5/5, 5/5, 5/5 |

- **Les lectures qui changent de longueur passent comme les autres** : à 3 % d'insertions, de
  suppressions et de substitutions par base, les quatre échantillons sont restitués à chaque
  tirage, en couverture 10 comme en 20 — là où, hors bibliothèque, `fountain`, `strands` et
  `goldman` échouent dès 1,5 %.
- **Chaque rendement a son taux** : à 3 % d'erreurs de synthèse, l'échantillon au rendement ¾
  échoue à chaque tirage — trois fois ce qu'il corrige —, et les trois autres passent.
- **Les lectures étrangères ne corrompent rien, mais elles coûtent** : à 10 % de sauts d'index,
  7 911 paires sont rangées dans un autre échantillon que le sien, et chaque archive
  est pourtant restituée. Les lectures du rendement de même longueur de brin y vont jusqu'au bout
  de leur budget.
- **La longueur de lecture doit couvrir le brin** : à 200 bases, les échantillons aux brins de
  300 nt (¼ et ⅓) échouent, ceux de 200 nt passent ; à 150, tous échouent.
- **Durée** (même machine) : 0,4 s par tirage, du séquenceur au décodage. Les lectures 2 des
  brins de 300 nt s'arrêtent dans l'insert : `adn demux` les écarte, et le décodeur ne les
  cherche plus en vain — 5,3 s par tirage avant.

## Limites

- **Lectures partielles** : elles ne s'assemblent pas. La longueur de lecture doit atteindre le
  plus long insert du pool — et 19 bases de plus pour que la lecture 2 couvre un insert entier.
  En dessous, la lecture 2 est écartée, et la lecture 1 reste rejetée par les décodeurs.
- **Format Nextera seul** : les adaptateurs en Y et le multiplexage par amorces ne sont pas
  décrits ; les molécules se construisent avec deux index, et le rangement sur le seul index i7
  ne s'applique qu'à des lectures.
- **Qualités ignorées** : les décodeurs ne lisent pas les qualités FASTQ ; elles sont contrôlées
  par leur longueur, puis écartées.
- **Un séquenceur simplifié** : ni doublons de PCR, ni amas mixtes, ni contrôle PhiX ; l'erreur
  croît le long de la lecture, sans biais de substitution ; le donneur d'un saut d'index est
  tiré au hasard parmi les molécules.
- **Serveur web** : les bibliothèques passent par la ligne de commande.
