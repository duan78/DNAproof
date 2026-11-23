# Spécification du format d'archive « ADN2 », version 2

Cette page décrit précisément ce que produit et lit la version 0.2 du projet. Chaque archive se
suffit à elle-même : elle se décode depuis ses seules séquences, sans en-tête FASTA, sans nom de
fichier et sans paramètre extérieur.

Toutes les conventions décrites ici sont figées : une archive écrite aujourd'hui doit rester
lisible par les versions à venir. L'algorithme pseudo-aléatoire en fait partie.

## 1. Garanties

1. **Rien n'est rendu sans vérification.** L'empreinte SHA-256 du fichier d'origine voyage dans
   l'archive. Un fichier n'est écrit que si l'empreinte du contenu reconstitué est identique.
   Une archive abîmée produit une erreur, jamais un contenu faux.
2. **Aucune information extérieure.** Le schéma, la compression, la longueur du corps et les
   paramètres de codage sont portés par des oligos « manifestes », au début de l'archive.
3. **Les oligos respectent les contraintes biochimiques.** Par défaut : 45 à 55 % de GC, et
   jamais plus de trois bases identiques de suite. Un candidat qui ne les respecte pas est
   écarté.

## 2. Vue d'ensemble

```
fichier
  │  compression, longueurs, SHA-256, CRC-32
  ▼
enveloppe (60 octets d'en-tête + données stockées)
  │  découpée en blocs, répartie dans des oligos
  ▼
oligos de données ── ┐
                    ├──► archive : oligos manifestes, puis oligos de données
oligos manifestes ──┘
```

À l'encodage, le fichier est d'abord enveloppé ([section 4](#4-enveloppe)). L'enveloppe, ou
*corps*, est répartie dans des oligos selon un schéma : [DNA Fountain](#6-schéma-dna-fountain)
ou [brins codés](#7-schéma-brins-codés). Enfin, 12 à 64 oligos manifestes
([section 5](#5-manifeste)) sont placés en tête.

Au décodage, l'ordre est inverse : reconnaître le manifeste, reconstituer le corps selon son
schéma, puis vérifier et rouvrir l'enveloppe.

## 3. Conventions

- **Octets en ADN** : deux bits par nucléotide, poids fort en tête, dans l'ordre
  `A = 00`, `C = 01`, `G = 10`, `T = 11`. Un oligo de *n* octets compte donc 4·*n* nucléotides.
- **Boutisme** : il dépend du champ ; chaque tableau le précise. En règle générale, les
  longueurs et les compteurs sont en petit-boutiste, la graine d'une goutte en gros-boutiste.
- **CRC-32** : IEEE 802.3 (polynôme réfléchi `0xEDB88320`), état initial et final `0xFFFFFFFF`.
- **SHA-256** : empreinte brute de 32 octets.
- **Reed-Solomon** : code systématique sur GF(2⁸), construit sur le polynôme primitif
  x⁸ + x⁴ + x³ + x² + 1 (`0x11D`). Un bloc codé s'écrit `données || parité`, avec *e* octets de
  parité, et compte 255 octets au plus. Il corrige jusqu'à ⌊*e*/2⌋ octets erronés, ou *e*
  effacements dont les positions sont connues.
- **Générateur pseudo-aléatoire** : voir la [section 10](#10-générateur-pseudo-aléatoire). Tous
  les masques et tous les tirages en dérivent.

## 4. Enveloppe

L'enveloppe est le corps de l'archive, tel qu'il est réparti dans les oligos.

| décalage | taille | contenu |
|----------|--------|---------|
| 0        | 4      | magic `ADN2` |
| 4        | 1      | version du format (`2`) |
| 5        | 1      | compression : `0` aucune, `1` LZ4, `2` zstd |
| 6        | 2      | réservé (`0`) |
| 8        | 8      | longueur du fichier original (petit-boutiste) |
| 16       | 8      | longueur des données stockées (petit-boutiste) |
| 24       | 32     | SHA-256 du fichier original |
| 56       | 4      | CRC-32 de l'en-tête (56 octets) suivi des données stockées |
| 60       | …      | données stockées : le fichier, compressé ou non |

- **LZ4** : bloc brut au format de la bibliothèque `lz4_flex` (fonction
  `compress_prepend_size`), précédé de sa longueur décompressée (4 octets, petit-boutiste).
- **zstd** : trame zstd, niveau 3.
- **Compression automatique** : zstd si le résultat est plus court que le fichier, aucune sinon.
  Le choix retenu est inscrit dans l'en-tête et n'a jamais à être deviné.

Le décodage vérifie le magic, la version, la cohérence des longueurs et le CRC-32 avant de
décompresser, puis l'empreinte SHA-256 du fichier obtenu.

Un fichier vide est une enveloppe de 60 octets ; il occupe quand même un bloc.

## 5. Manifeste

Un oligo manifeste a la même longueur que les oligos de données du même schéma :

| étiquette `ADNF`, 4 octets | nonce, 4 octets | manifeste masqué, 28 octets | parité Reed-Solomon |
|---------------------------|-----------------|------------------------------|---------------------|

Avec DNA Fountain et sa parité par défaut, cela fait 36 + 2 = 38 octets, soit 152 nt ; avec les
brins codés, 36 + 4 = 40 octets, soit 160 nt. Le nombre d'octets de parité est celui du schéma.

- **Étiquette** : les quatre octets ASCII `ADNF`, en clair, soit les 16 premiers nucléotides
  `CAACCACACATGCACG`. Elle permet de reconnaître un oligo manifeste parmi les lectures.
- **Nonce** : un numéro de série, de 4 octets en petit-boutiste, transmis sous une forme
  dispersée par [`scramble_u32`](#10-générateur-pseudo-aléatoire).
- **Manifeste masqué** : le manifeste ci-dessous, dont chaque octet est combiné par OU exclusif
  avec un flux pseudo-aléatoire dérivé du nonce transmis (flux `0x10`). Le masquage évite
  qu'un contenu très structuré — des octets nuls — ne donne en ADN de longues suites de A,
  rejetées par le criblage.
- **Parité** : calculée sur l'ensemble étiquette, nonce et manifeste *avant* masquage. Le
  décodeur retire le masque, puis corrige.

Le manifeste compte 28 octets :

| décalage | taille | contenu |
|----------|--------|---------|
| 0        | 1      | version du format (`2`) |
| 1        | 1      | schéma : `1` = DNA Fountain, `2` = brins codés |
| 2        | 1      | octets de parité Reed-Solomon de chaque oligo du schéma |
| 3        | 1      | compression de l'enveloppe |
| 4        | 8      | longueur de l'enveloppe, en octets (petit-boutiste) |
| 12       | 4      | nombre de blocs (DNA Fountain), `0` sinon |
| 16       | 4      | `c` de la distribution soliton (flottant 32 bits, petit-boutiste) |
| 20       | 4      | `δ` de la distribution soliton (flottant 32 bits, petit-boutiste) |
| 24       | 4      | CRC-32 des 24 premiers octets |

`c` et `δ` sont arrondis en flottants 32 bits avant l'encodage : l'encodeur travaille avec ces
valeurs arrondies, exactement celles que le décodeur relira.

**Nombre de copies.** Un dixième des oligos de données, au moins 12 et au plus 64. Les copies
portent le même manifeste et diffèrent par leur nonce. Chacune est cherchée à partir du nonce
`997 × rang de la copie` : le premier nonce dont l'oligo respecte les contraintes est retenu.

**Lecture.**

1. Une lecture dont les 16 premiers nucléotides diffèrent d'au plus un groupe de quatre de
   l'étiquette est retenue comme candidate. Les autres lectures sont des lectures de données.
2. Le nombre d'octets de parité se déduit de la longueur de la lecture : longueur en octets
   moins 36.
3. Chaque candidate est démasquée avec son nonce, puis corrigée par Reed-Solomon. L'étiquette,
   la version et le CRC-32 du manifeste sont contrôlés.
4. Une erreur dans le nonce fausse le masque. Si la lecture directe échoue, les 48 variantes du
   nonce à une base près sont essayées (16 nucléotides, 3 remplacements chacun).

Le manifeste retenu est celui que rassemble le plus de lectures strictement identiques ; en cas
d'égalité, le premier rencontré l'emporte. Un seul manifeste lisible suffit : il est protégé par
sa parité et son CRC-32, et l'empreinte SHA-256 du fichier reste l'arbitre final.

## 6. Schéma DNA Fountain

DNA Fountain (Erlich et Zielinski, 2017) répartit le corps en *gouttes* sans ordre ni adresse :
l'archive se reconstitue à partir de n'importe quelles lectures, dans n'importe quel ordre.

### Oligo

| graine, 4 octets | charge utile masquée, 32 octets | parité Reed-Solomon |
|------------------|---------------------------------|---------------------|

Par défaut la parité compte 2 octets, comme dans l'article : l'oligo fait 38 octets, soit
152 nt. La parité peut être portée jusqu'à 32 octets (voir [paramètres](#paramètres)).

1. Le corps est complété par des octets nuls jusqu'à un multiple de 32 octets, puis coupé en
   *K* blocs de 32 octets. *K* vaut au moins 1.
2. Les gouttes sont numérotées `0, 1, 2…`. La **graine** d'une goutte est son numéro passé dans
   la bijection [`scramble_u32`](#10-générateur-pseudo-aléatoire), inscrite en gros-boutiste.
3. Le **degré** *d* de la goutte est tiré de la distribution soliton robuste avec le flux `1`.
4. Les *d* blocs qu'elle mélange sont tirés sans remise par l'algorithme de Floyd, avec le flux
   `2`, et rangés par index croissant.
5. La **charge utile** est le OU exclusif de ces blocs. Elle est masquée par le flux `0x13`
   dérivé de la graine.
6. La parité Reed-Solomon protège `graine || charge utile masquée`.
7. Le bloc est converti en ADN. **S'il ne respecte pas les contraintes, la goutte est écartée** et
   on passe au numéro suivant.

### Lecture d'une goutte

1. Seules les lectures de la longueur attendue sont retenues.
2. Reed-Solomon corrige chaque lecture, ou l'écarte.
3. La graine lue dans le bloc sert à recalculer degrés et indices, et à retirer le masque.
4. Une goutte par graine. Si plusieurs lectures portent la même graine, les lectures reçues sans
   erreur — parité vérifiée du premier coup — l'emportent sur les lectures « corrigées » : avec
   deux octets de parité, deux octets erronés peuvent se corriger vers un bloc faux (une fois sur
   sept environ).
5. Entre lectures reçues sans erreur d'une même graine, la charge utile qui rassemble le plus de
   voix l'emporte ; à égalité, la goutte est écartée. Ce vote n'est pas une précaution vaine : la
   distance minimale du code vaut 3, et trois substitutions bien placées forment un autre mot de
   code, que la parité reçoit comme exact. Sans aucune lecture sans erreur, les lectures
   corrigées ne comptent que si elles s'accordent toutes.

### Décodage

**1. Le vote.** Pour chaque graine, les lectures reçues sans erreur votent, et la charge utile
qui rassemble le plus de voix l'emporte ; une égalité écarte la goutte.

- Une goutte est **certaine** dès que deux lectures concordent : deux lectures fausses de même
  graine qui s'accordent, cela n'arrive pas.
- Avec une seule lecture, elle n'est que **probable**.
- Sans aucune lecture reçue sans erreur, les lectures corrigées ne comptent que si elles
  s'accordent toutes : la goutte est probable.

**2. La résolution.** Les gouttes sont « épluchées » : une goutte de degré 1 révèle son bloc, qui
est soustrait des autres gouttes qui le mélangent, et ainsi de suite jusqu'à ce que tous les
blocs soient connus. L'épluchage commence avec les seules gouttes certaines : leur solution est
exacte, et une goutte probable ne peut pas la corrompre. S'il manque des blocs, les gouttes
probables complètent — et leur solution reste à vérifier. Le résultat ne dépend pas de l'ordre des
lectures.

**3. La vérification.** Si le corps reconstitué ne passe pas le contrôle d'intégrité de
l'enveloppe, une lecture « corrigée » l'a probablement été à tort. Un second passage recommence
sans jamais utiliser une lecture corrigée. L'empreinte SHA-256 reste l'arbitre final.

Ces précautions ne sont pas vaines. La distance minimale du code vaut 3 : trois substitutions bien
placées forment un autre mot de code, que la parité reçoit comme exact. Une seule de ces lectures,
prise pour une goutte, corrompait des milliers de blocs.

### Paramètres

| Paramètre | Défaut | Rôle |
|-----------|--------|------|
| `redundancy` | `1.10` | marge de perte : `r` signifie que l'archive reste décodable après la perte aléatoire d'une fraction `1 − 1/r` des oligos |
| `c` | automatique | paramètre de la distribution soliton |
| `delta` | `0.001` | paramètre de la distribution soliton |
| `rs_parity` | `2` | octets de parité par oligo (1 à 32) |

**Choix automatique de `c`.** La distribution réserve `S = c · ln(K/δ) · √K` gouttes de degré 1
pour `K` blocs. Un bloc n'en reçoit aucune avec une probabilité proche de e^(−S) : il faut donc
`S` bien plus grand que `ln(K)`. La valeur de l'article (`c = 0,025`) y pourvoit dès quelques
milliers de blocs. En dessous, `c` est relevé pour garder `S ≥ ln(K/δ) + 6`, dans la limite
de `c ≤ 1`.

**Certification de la marge.** L'encodeur tient un décodeur à jour et ajoute des gouttes
jusqu'à ce que l'archive reste décodable après l'écart aléatoire de la fraction `1 − 1/r` des
oligos, pour chacun de 256 tirages indépendants (96 au-delà de 50 000 blocs, 32 au-delà de
500 000). Un code LT a une queue de distribution lourde : sans cette certification, une archive
à peine décodable échoue une fois sur dix dès qu'on lui retire des oligos.

**Conséquence sur les petits fichiers.** Pour quelques dizaines ou centaines de blocs, la
distribution soliton réclame nettement plus d'une goutte par bloc : un fichier de 4 Ko donne
environ 2,9 gouttes par bloc, et une densité d'environ 0,5 bit par nucléotide.

### Distribution soliton robuste

Définition de Luby (2002), telle que mise en œuvre dans le logiciel de référence de DNA
Fountain, pour `K` blocs :

- `S = c · ln(K/δ) · √K` ; pivot `⌊K/S⌋` ;
- `ρ(1) = 1/K`, `ρ(d) = 1/(d·(d−1))` ;
- `τ(d) = (S/K)/d` pour `d < pivot`, `τ(pivot) = (S/K) · ln(S/δ)`, nul ensuite ;
- `μ(d) = (ρ(d) + τ(d)) / Z`, où `Z` normalise la somme à 1.

Le degré est tiré en cherchant, dans la fonction de répartition cumulée, le premier indice dont
la valeur dépasse un réel uniforme de `[0, 1)`.

### Taille maximale

Le nombre de blocs tient sur 32 bits : le corps peut atteindre (2³² − 1) × 32 octets, soit
environ 137 Gio. La limite pratique vient avant : l'encodage demande environ cinquante fois la
taille du fichier en mémoire ([performances](PERFORMANCE.md)).

## 7. Schéma brins codés

Chaque brin porte son adresse, protégée ; un code externe répare les brins manquants.
L'architecture à deux niveaux de codes suit celle de Grass et al. (2015). Les paramètres
diffèrent de l'article.

### Brin

| nonce, 3 octets | index, 3 octets | charge utile, 30 octets | parité interne, 4 octets |
|-----------------|-----------------|-------------------------|--------------------------|

Un brin fait 40 octets, soit 160 nt.

- **Index** : rang du brin dans l'archive, 3 octets en petit-boutiste.
- **Mot de code** : `index || charge utile`, auquel s'ajoutent 4 octets de parité interne.
  Il corrige jusqu'à 2 octets erronés par lecture.
- **Masque** : le mot de code de 37 octets est masqué par le flux `0x12`, dérivé du nonce
  transmis. La parité est calculée avant masquage.
- **Nonce** : les nonces sont essayés à partir de `0`, et le premier qui donne un brin conforme
  aux contraintes est retenu. Il est transmis sous une forme dispersée par
  [`scramble_u24`](#10-générateur-pseudo-aléatoire), en petit-boutiste. Les 24 bits offrent plus
  de 16 millions de candidats par brin.

### Code externe

Le corps est complété par des octets nuls jusqu'à un multiple de 30 octets et coupé en brins de
données. Ceux-ci sont groupés par blocs de 223. Chaque bloc reçoit 32 **brins de parité**, qui
suivent tous les brins de données et sont indexés comme eux :

```
index d'un brin de parité = nombre de brins de données + 32 × rang du bloc + rang de la parité
```

Pour chacante des 30 colonnes d'un bloc, la parité est celle d'un code Reed-Solomon à 32 octets
calculé sur les valeurs de cette colonne dans les 223 brins du bloc. Le brin de parité `p` du
bloc porte, à sa colonne *i*, le *p*-ième octet de parité de la colonne *i*. Le code externe
répare jusqu'à 32 brins manquants par bloc, soit 12,5 % des 255 brins du bloc.

Le code externe sert aussi à contrôler chaque bloc. Un brin manquant est un effacement et un
octet faux une erreur ; colonne par colonne, il corrige tout motif tel que
`2 · erreurs + effacements ≤ 32`. Un brin faux compte donc pour deux brins manquants. Ainsi un
brin présent, mais lu à tort — une lecture unique, corrigée à tort par le code interne, ne peut
être reconnue sur sa seule parité — est-il rétabli, ou l'échec est-il annoncé. Au-delà de la
capacité du code, un décodeur Reed-Solomon ne promet pas d'échouer : il peut se tromper en
silence. Le CRC-32 puis l'empreinte SHA-256 de l'enveloppe restent les arbitres.

### Lecture d'un brin

Si la lecture directe échoue, les 36 variantes du nonce à une base près sont essayées (12
nucléotides, 3 remplacements chacun). Les lectures dont l'index dépasse celui de l'archive sont
écartées.

Les lectures d'un même index doivent s'accorder sur la charge utile. Si les lectures discordantes
sont au moins aussi nombreuses que les concordantes, le brin est écarté : le code externe le
répare, s'il en a la capacité.

### Taille maximale

L'index tient sur 24 bits : une archive compte moins de 2²⁴ brins, données et parité
confondues. Le corps peut ainsi atteindre environ 440 Mo (420 Mio).

## 8. Fichier FASTA

Une archive se écrit dans un fichier FASTA ordinaire, une séquence repliée à 60 nucléotides par
ligne. **Les en-têtes sont purement informatifs** : le décodage ne les lit pas.

```
>adn-fountain-v2 manifest=0
CAACCACACATGCACGTAAAGCAAGAGCCGGAAGTTATACACTGACGCTCCTAACGTAAT
...
>adn-fountain-v2 oligo=0
...
```

- Les manifestes viennent en tête : `adn-<schéma>-v2 manifest=<rang>`.
- Les oligos de données suivent : `adn-<schéma>-v2 oligo=<rang>`.
- `<schéma>` vaut `fountain` ou `strands`.

La lecture tolère les fins de ligne CRLF, les lignes repliées, les minuscules, les lignes vides
et les lignes de commentaire commençant par `;`. Elle refuse les chiffres et la ponctuation
dans les séquences. Les codes d'ambiguïté tels que `N` sont acceptés à la lecture ; les décodeurs
les écartent et les comptent.

Plusieurs fichiers de lectures peuvent être donnés au décodage : leurs séquences sont
concaténées.

## 9. Décodage

1. **Normalisation.** Majuscules et blancs tolérés ; tout autre caractère écarte la lecture.
2. **Manifeste.** Il donne le schéma et les paramètres de l'archive
   ([section 5](#5-manifeste)).
3. **Corps**, selon le schéma ([6](#6-schéma-dna-fountain) ou [7](#7-schéma-brins-codés)).
4. **Enveloppe.** CRC-32, décompression, puis comparaison de l'empreinte SHA-256.

Un schéma attendu peut être imposé ; une archive d'un autre schéma est alors refusée.

## 10. Générateur pseudo-aléatoire

Le décodeur recalcule la structure des gouttes en rejouant les tirages de l'encodeur.
L'algorithme fait donc partie du format : il est figé, et il ne doit jamais changer.

Il s'agit de **SplitMix64**, complété par deux échantillonneurs.

- Constante γ = `0x9E3779B97F4A7C15`.
- Fonction de mélange `mix(z)` : `z ^= z >> 30` ; `z *= 0xBF58476D1CE4E5B9` ;
  `z ^= z >> 27` ; `z *= 0x94D049BB133111EB` ; `z ^= z >> 31` (produits modulo 2⁶⁴).
- Ouverture du flux numéro `s` pour la graine `g` : état initial `mix(g XOR (s · γ))`.
- Entier de 64 bits : l'état est incrémenté de γ, et le résultat est `mix(état)`.
- Entier de 32 bits : les 32 bits de poids fort d'un entier de 64 bits.
- Octets : chaque entier de 64 bits est écrit en petit-boutiste.
- Réel de `[0, 1)` : `(x >> 11) × 2⁻⁵³`.
- Entier uniforme de `[0, n)` : méthode de Lemire, `x · n` calculé sur 128 bits, avec rejet
  sous le seuil `(−n) mod n`.

Deux usages distincts ne partagent jamais le même numéro de flux :

| Flux   | Usage |
|--------|-------|
| `0x01` | degré d'une goutte |
| `0x02` | blocs mélangés par une goutte |
| `0x10` | masque du manifeste |
| `0x11` | tirages de perte de la certification |
| `0x12` | masque d'un brin codé |
| `0x13` | masque de la charge utile d'une goutte |

**`scramble_u32`** disperse un compteur sur 32 bits, pour les graines et les nonces de
manifeste :

```
x ^= x >> 16 ;  x *= 0x7FEB352D ;
x ^= x >> 15 ;  x *= 0x846CA68B ;
x ^= x >> 16
```

**`scramble_u24`** disperse un compteur sur 24 bits, pour les nonces des brins codés. Chaque
étape est une bijection sur 24 bits, et les produits sont pris modulo 2²⁴ :

```
x = (x ^ (x >> 11) ^ 0x5A3C7E) ;
x = x · 0x800015 ;
x ^= x >> 13 ;
x = x · 0x6F4F2D ;
x ^= x >> 12
```

Les deux bijections donnent à un compteur une apparence aléatoire : un compteur brut
s'écrirait en ADN comme de longues suites de A, rejetées par le criblage. L'implémentation de
référence est le fichier `crates/core/src/codec/rng.rs`.

## 11. Limites récapitulatives

| | DNA Fountain | Brins codés |
|---|---|---|
| Oligo | 152 nt (par défaut) | 160 nt |
| Corps maximal | ≈ 137 Gio | ≈ 420 Mio |
| Sans ordre | oui | non : les brins sont adressés |
| Perte aléatoire supportée | `1 − 1/r` des oligos, par construction (`r` = marge) | 32 brins manquants sur 255, soit 12,5 % d'un bloc |
| Substitutions | dépend de la parité : avec 2 octets, une seule erreur par lecture est corrigée | 2 octets erronés par brin |
| Limite pratique | vitesse d'encodage | taille |

## 12. Version 1

Les archives produites par la version 0.1 ne sont plus lues : leurs en-têtes portent la marque
`|scheme:`, et les schémas qu'elles annonçaient ne mettaient pas en œuvre ce que leur nom
promettait. Les noms `goldman`, `goldman2013`, `grass2015`, `adaptive`, `base3` et `ultimate`
sont refusés avec un message explicite. Pour relire une telle archive, utiliser le code du
commit `09df4f8`.

## Références

- Erlich Y., Zielinski D. — *DNA Fountain enables a robust and efficient storage architecture*,
  Science 355(6328):950–952, 2017.
- Luby M. — *LT codes*, FOCS 2002.
- Grass R. N., Heckel R., Puddu M., Paunescu D., Stark W. J. — *Robust Chemical Preservation of
  Digital Information on DNA in Silica with Error-Correcting Codes*, Angewandte Chemie
  International Edition 54:2552–2555, 2015.
