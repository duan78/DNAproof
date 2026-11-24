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
   écarté. Une exception, fidèle à son article : le schéma goldman garantit l'absence de bases
   répétées, mais pas la teneur en GC de ses brins (50 % en moyenne).

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
*corps*, est répartie dans des oligos selon un schéma : [DNA Fountain](#6-schéma-dna-fountain),
[brins codés](#7-schéma-brins-codés), [goldman](#8-schéma-goldman) ou
[grass](#9-schéma-grass) ou [ldpc](#10-schéma-ldpc). Enfin, 12 à 64 oligos manifestes ([section 5](#5-manifeste)) sont
placés en tête.

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
- **Trits et symboles de GF(47)** : les schémas goldman et grass n'écrivent pas deux bits par
  nucléotide. L'un écrit un trit (chiffre de 0 à 2) par base, l'autre un symbole (de 0 à 46) par
  groupe de trois bases. Leurs conventions sont données dans leur section.
- **Générateur pseudo-aléatoire** : voir la [section 13](#13-générateur-pseudo-aléatoire). Tous
  les masques et tous les tirages en dérivent.
- **Rapports de vraisemblance** : le schéma ldpc ne tranche pas base à base ; il pèse chaque
  bit du rapport `ln P(bit = 0) / P(bit = 1)`, positif pour un bit à zéro, nul pour un effacement.

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

Un oligo manifeste est toujours écrit en deux bits par nucléotide :

| étiquette `ADNF`, 4 octets | nonce, 4 octets | manifeste masqué, 28 octets | parité Reed-Solomon |
|---------------------------|-----------------|------------------------------|---------------------|

Avec DNA Fountain et sa parité par défaut, cela fait 36 + 2 = 38 octets, soit 152 nt ; avec les
brins codés, 36 + 4 = 40 octets, soit 160 nt ; avec grass et sa parité par défaut, 36 + 14 = 50
octets, soit 200 nt ; avec goldman ou ldpc, 152 nt. Le nombre d'octets de parité est celui du
schéma (deux pour goldman et ldpc, dont les brins n'en portent pas). Les manifestes du schéma
grass sont donc plus longs que ses brins, qui portent autre chose que deux bits par nucléotide.

- **Étiquette** : les quatre octets ASCII `ADNF`, en clair, soit les 16 premiers nucléotides
  `CAACCACACATGCACG`. Elle permet de reconnaître un oligo manifeste parmi les lectures.
- **Nonce** : un numéro de série, de 4 octets en petit-boutiste, transmis sous une forme
  dispersée par [`scramble_u32`](#13-générateur-pseudo-aléatoire).
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
| 1        | 1      | schéma : `1` = DNA Fountain, `2` = brins codés, `3` = goldman, `4` = grass, `5` = ldpc |
| 2        | 1      | parité Reed-Solomon de chaque oligo : en octets (DNA Fountain, brins codés), en symboles (grass), `2` pour goldman et ldpc (leurs manifestes) |
| 3        | 1      | compression de l'enveloppe |
| 4        | 8      | longueur de l'enveloppe, en octets (petit-boutiste) |
| 12       | 4      | nombre de blocs (DNA Fountain) ou de fenêtres (goldman), `0` sinon |
| 16       | 4      | `c` de la distribution soliton (flottant 32 bits, petit-boutiste) ; pour goldman, l'identifiant de fichier dans le premier octet ; pour ldpc, la parité en pour cent dans le premier octet ; le reste à zéro |
| 20       | 4      | `δ` de la distribution soliton (flottant 32 bits, petit-boutiste), `0` sinon |
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
   la bijection [`scramble_u32`](#13-générateur-pseudo-aléatoire), inscrite en gros-boutiste.
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
  [`scramble_u24`](#13-générateur-pseudo-aléatoire), en petit-boutiste. Les 24 bits offrent plus
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

## 8. Schéma goldman

Goldman et al. (2013) : les octets deviennent des trits, chaque trit choisit une base différente
de la précédente, et l'ADN est relu par des fenêtres qui se recouvrent — chaque base du message
est lue quatre fois, dans quatre brins. La méthode est celle de l'information complémentaire de
l'article ; deux adaptations sont signalées.

### Brin

| repère, 1 nt | message, 100 nt | index, 15 nt | repère, 1 nt |
|--------------|-----------------|--------------|--------------|

Un brin fait 117 nt. Il ne contient jamais deux bases identiques de suite.

### Message ternaire

1. **Octets en trits.** Deux codes préfixe ternaires sont possibles, annoncés par un premier
   octet (le *mode*), lui-même écrit avec le code fixe :
   - `0` : le **code fixe**, pour tout le corps — les octets 0 à 235 s'écrivent en 5 trits, les
     octets 236 à 255 en 6 ;
   - `1` : le **code de Huffman de l'archive** — ses 256 longueurs (un octet chacune, de 0 pour
     un octet absent à 80 au plus) suivent le mode, écrites avec le code fixe, puis le corps est
     écrit avec ce code.

   Le code de Huffman est celui de l'algorithme de Huffman sur les comptes d'octets du corps, en
   base 3 (trois nœuds fusionnés à la fois, une feuille de poids nul ajoutée quand il le faut).
   Dans les deux cas le code est **canonique** : les symboles sont rangés par longueur puis par
   valeur, et le premier code de longueur *L* vaut `(premier code de L − 1 + nombre de codes de
   L − 1) × 3`. Un code de *L* trits s'écrit poids fort en tête. Le code de Huffman n'est retenu
   que si, table comprise (1 285 trits), il donne un filet plus court que le code fixe. Un mot de
   6 trits du code fixe n'appartient à aucun octet : il signale une lecture fausse.
2. **Longueur et bourrage.** Le filet des octets (*S₁*) est suivi de zéros (*S₃*), puis de la
   longueur de *S₁* sur 20 trits, poids fort en tête (*S₂*) — soit *S₄ = S₁ ‖ S₃ ‖ S₂*, de
   longueur multiple de 25 et d'au moins 100.
3. **Trits en bases.** Le premier trit part de la ligne `A` ; chacun choisit l'une des trois
   bases différentes de la précédente :

   | précédente | trit 0 | trit 1 | trit 2 |
   |------------|--------|--------|--------|
   | A          | C      | G      | T      |
   | C          | G      | T      | A      |
   | G          | T      | A      | C      |
   | T          | A      | C      | G      |

### Fenêtres

L'ADN obtenu (*S₅*, *N* bases) est entouré de 75 nt de bourrage de chaque côté : à gauche la
suite `CGA` répétée 25 fois, à droite la suite des trits nuls qui suit la dernière base. Chaque
position du message est ainsi couverte par quatre fenêtres, la première et la dernière
comprises (adaptation : l'article n'a pas de bourrage, et ses extrémités ne sont couvertes qu'une
à trois fois).

- La fenêtre *i* est `bourré[25·i … 25·i + 100)` : il y en a `N/25 + 3`, numérotées à partir de
  0. Le manifeste en porte le nombre.
- Une fenêtre **impaire** est complémentée en sens inverse.
- L'**index** de 15 trits suit le message, converti par la même table à partir de sa dernière
  base : l'identifiant de fichier (2 trits), l'adresse de la fenêtre (12 trits, poids fort en
  tête), puis un trit de parité `P`. Celui-ci est la somme, modulo 3, du premier trit de
  l'identifiant et des trits d'adresse de rang pair dans l'écriture (le premier trit compte) :
  `P = ID₁ + i₃₁ + i₃₃ + i₃₅ + i₃₇ + i₃₉ + i₃₁₁`. L'identifiant de fichier vaut
  `identifiant / 3` puis `identifiant mod 3` ; il va de 0 à 8 et sépare plusieurs archives
  mélangées dans un même tube.
- Les deux **bases de repère** encadrent le tout : `A` ou `T` en tête, `C` ou `G` en fin,
  différentes de la base voisine. Quand le choix est libre, la tête vaut `A` si le bit 0 de
  `scramble_u32(i)` est nul et `T` sinon ; la fin vaut `C` si le bit 1 est nul et `G` sinon. Un
  même fichier donne donc toujours les mêmes brins.

### Lecture d'un brin

1. Seules les lectures de 117 nt sont retenues.
2. **Sens.** Un brin commence par `A` ou `T` et finit par `C` ou `G` ; lu dans l'autre sens, il
   commence par `G` ou `C` et finit par `T` ou `A`. Si aucune des deux règles ne s'applique, les
   deux sens sont essayés : l'index tranche.
3. **Répétitions.** Deux bases identiques de suite sont impossibles : l'erreur est sur l'une des
   deux, et les deux positions sont écartées du vote. La première base du message l'est aussi si
   elle répète la base de repère.
4. **Index.** L'identifiant doit être celui du manifeste, l'adresse inférieure au nombre de
   fenêtres, et la parité exacte. Toute substitution dans l'index est détectée : les trits qu'elle
   change sont deux voisins, dont l'un au moins est couvert par la parité.
5. La fenêtre impaire est remise dans le sens du message.

### Décodage

Chaque position de l'ADN encodé reçoit une voix par lecture qui la couvre. La base qui rassemble
le plus de voix l'emporte ; une égalité, ou une position sans voix, arrête le décodage. L'ADN
est ensuite relu en trits, la longueur et le bourrage contrôlés, et le message décodé.

Le schéma n'a pas de code correcteur : sa robustesse vient du recouvrement. Il demande donc de la
couverture, et une erreur dans l'index d'un brin le fait perdre. Il ne règle pas non plus la
teneur en GC de ses brins (50 % en moyenne) : l'encodage n'admet que les bornes de GC par défaut,
ou des bornes ouvertes de 0 à 1.

### Taille maximale

L'adresse tient sur 12 trits : 531 441 fenêtres au plus, soit un ADN encodé de 13 285 950 nt et
un corps d'environ 2,6 Mo avec le code fixe.

## 9. Schéma grass

Grass et al. (2015) : l'information est portée par des symboles du corps à 47 éléments, corrigés
par du Reed-Solomon entrelacé par blocs de 713 brins × 39 caractères, et chaque symbole s'écrit en
trois nucléotides, choisis pour qu'aucune suite de plus de trois bases identiques n'apparaisse.
L'adressage logique (l'index) et la forme du code externe sont des ajouts du format v2 : l'article
adressait chaque brin par ses amorces.

### Trimères

Parmi les 64 trimères possibles, 47 sont retenus, dans cet ordre (le symbole est leur rang) :

1. les 36 trimères sans deux bases adjacentes égales, par ordre lexicographique `A < C < G < T`
   (symboles 0 à 35) ;
2. 11 des 12 trimères terminés par un doublet, dans le même ordre ; `CAA` est écarté, pour que le
   jeu reste équilibré en GC (symboles 36 à 46).

Deux trimères quelconques mis bout à bout ne comptent jamais plus de trois bases identiques. Les
17 trimères écartés (les homopolymères, les trimères à doublet initial, et `CAA`) n'apparaissent
pas dans un brin conçu : le décodeur y voit un effacement.

### Symboles et octets

Un groupe de *P* symboles porte *B* octets, *B* étant le plus grand nombre tel que
`256^B ≤ 47^P` : les octets, poids fort en tête, sont écrits en base 47, poids fort en tête.

| Parité interne *R* | Symboles de charge *P* | Octets par brin *B* |
|--------------------|------------------------|---------------------|
| 30                 | 4                      | 2                   |
| 20                 | 14                     | 9                   |
| 14 (défaut)        | 20                     | 13                  |
| 10                 | 24                     | 16                  |
| 6                  | 28                     | 19                  |
| 4                  | 30                     | 20                  |
| 2                  | 32                     | 22                  |

### Brin

| index, 4 symboles | nonce, 1 symbole | charge utile, P symboles | parité interne, 39 − 5 − P symboles |
|-------------------|------------------|--------------------------|-------------------------------------|

Un brin fait 39 symboles, soit 117 nt.

- **Index** : le numéro du brin en base 47, 4 chiffres poids fort en tête. Sur le fil, chaque
  chiffre est masqué par un ajout modulo 47 : `23, 7, 41, 15`. Sans ce masque fixe, tous les
  brins commenceraient par les mêmes trimères.
- **Nonce** : de 0 à 46, en clair.
- **Mot de code** : `index ‖ nonce ‖ charge utile`, auquel s'ajoutent `R` symboles de parité. Le
  code est un Reed-Solomon de longueur 39 et de dimension `5 + P` sur GF(47). Il corrige jusqu'à
  `R/2` symboles erronés par lecture, ou `R` effacements.
- **Masque** : le nonce, la charge utile et la parité (les 35 derniers symboles) sont masqués par
  le flux `0x14`, dérivé du nonce : chaque symbole devient `(symbole + tirage) mod 47`, les
  tirages étant les entiers de Lemire dans `[0, 47)`. La parité est calculée avant masquage.
- **Nonce** : essayés à partir de `0`, le premier qui donne un brin conforme aux contraintes est
  retenu. Il y en a 47 : l'encodage refuse d'emblée des contraintes dont le risque cumulé, sur
  l'archive, qu'aucun nonce ne convienne dépasse un millionième (une fenêtre de GC plus étroite
  que quelques pourcents, par exemple).

### Les deux corps

- **GF(47)** : les entiers modulo 47. Le générateur du groupe multiplicatif est 5.
- **GF(47²)** : les polynômes `a + b·x` à coefficients dans GF(47), pris modulo `x² − 5` (5
  n'est pas un carré modulo 47). L'élément `a` de GF(47) y est `a + 0·x`. Un générateur du
  groupe multiplicatif (2 208 éléments) est `x + 4` : le premier `x + c` dont les puissances
  1 104, 736 et 96 ne valent pas 1.

### Reed-Solomon d'évaluation

Sur un corps, un message est un polynôme de degré inférieur à *k*, et le mot de code ses valeurs
aux points `α⁰, α¹, … αⁿ⁻¹` (α est le générateur). Les *k* premières valeurs sont les données, les
suivantes la parité. Le code corrige tout motif de `2 · erreurs + effacements ≤ n − k`. Le
décodage suit Gao : interpolation rationnelle par algorithme d'Euclide étendu, les effacements
étant les points que l'interpolation ignore. Quand un même jeu d'effacements sert à plusieurs
colonnes, elles sont rétablies par interpolation directe, et le décodage complet ne prend le
relais que si une valeur conservée est fausse.

### Blocs et code externe

Un code de Reed-Solomon sur GF(47) ne dépasse pas 46 symboles : le code externe travaille sur
GF(47²), en prenant les symboles de charge utile deux par deux — c'est pourquoi la parité interne
est paire. Avec la parité par défaut, un brin porte 10 symboles de GF(47²).

Les brins sont groupés par blocs de 713, comme dans l'article (713 = 23 × 31) : les positions 0 à
620 sont des données, les positions 621 à 712 de la parité externe. Le numéro d'un brin vaut
`713 × rang du bloc + position`. Les places que le dernier bloc n'occupe pas comptent pour zéro.

Le corps est complété par des octets nuls, coupé en brins de *B* octets, puis écrit en symboles.
Pour chaque colonne de GF(47²) d'un bloc, la parité est celle d'un Reed-Solomon (713, 621),
calculée sur les valeurs de cette colonne dans les 621 brins de données. Le code externe
répare jusqu'à 92 brins manquants par bloc, soit 13 % ; colonne par colonne, il corrige tout
motif tel que `2 · erreurs + effacements ≤ 92`.

### Lecture d'un brin

1. Seules les lectures de 117 nt sont retenues ; chacune donne 39 symboles, un trimère écarté
   valant effacement.
2. **Nonce.** Si le nonce lu ne donne pas un mot de code qui le confirme — il est erroné, ou son
   trimère a été écarté —, les autres nonces sont essayés.
3. L'index, lu dans le mot de code corrigé, doit être dans l'archive.
4. Les lectures d'un même index doivent s'accorder sur la charge utile ; à égalité ou à la
   minorité, le brin est écarté et le code externe le répare, s'il en a la capacité.

### Paramètres

| Paramètre | Défaut | Rôle |
|-----------|--------|------|
| `parity`  | `14`   | symboles de parité interne par brin, pair, de 2 à 30 : un brin résiste à la moitié de symboles erronés, et sa charge utile diminue d'autant |

Le manifeste porte la parité interne dans son champ de parité ; l'oligo manifeste fait 4 nt de
plus par symbole.

### Taille maximale

L'index tient sur 47⁴ = 4 879 681 numéros : 6 843 blocs, soit 4 249 503 brins de données — 55 Mo
avec la parité par défaut, 81 Mo avec une parité de 6.
## 10. Schéma ldpc

Un schéma à décisions souples : deux codes en série sur l'image binaire du canal — un code
convolutif par brin, un LDPC par bloc —, reliés par un entrelacement. Il ne suit aucun article
en particulier : il met en œuvre pour l'archivage sur ADN le code convolutif le plus répandu,
celui des normes de communication spatiale, et les codes LDPC de Gallager.

### Brin

Un brin fait 154 nt, soit 308 bits de code, poids fort en tête deux par base :

| sel, 20 bits | index, 24 bits | charge utile, 104 bits (13 octets) | queue, 6 zéros |
|---|---|---|---|
| dispersé, en clair | dispersé, masqué par le sel | masquée par un flux dérivé de l'index et du sel | ramène l'encodeur à l'état nul |

Les 148 bits d'information entrent dans le **code convolutif binaire** de longueur de
contrainte 7 et de rendement ½, de polynômes générateurs (171, 133) en octal. À chaque bit
d'entrée, le registre vaut `(entrée << 6) | état`, où l'état compte les six entrées
précédentes, de la plus récente en poids fort à la plus ancienne ; le code donne
`parité(registre & 0o171)`, puis `parité(registre & 0o133)`, et l'état devient
`(état >> 1) | (entrée << 5)`. L'état initial est nul, et six zéros de queue ramènent l'encodeur
à zéro : le décodeur sait où finir. Un un isolé en entrée fait lire `1111001` puis `1011011`.
La distance libre du code vaut 10.

- **Sel** : un entier de 20 bits, transmis dispersé par la bijection
  `x ↦ ((x · 0x6E5D1 + 0x27F3A) mod 2²⁰) ⊕ (x >> 10)`. Il règle le criblage et, par là, tout le
  brin.
- **Index** : dispersé par `scramble_u24`, puis masqué par 24 bits dérivés du seul sel (flux
  `0x15`, graine = sel).
- **Charge utile** : masquée octet par octet par un flux dérivé de l'index et du sel (flux
  `0x14`, graine = `index · 2²⁰ + sel`).

### Blocs et code externe

Les brins se rangent par blocs : jusqu'à 819 brins de données, suivis de
⌈d · p / 100⌉ brins de parité, où `d` est le nombre de brins de données du bloc et `p` la
parité. Les brins d'un bloc se suivent dans l'espace des index : le premier du bloc *b* a le
rang *b* · (819 + ⌈819 · p / 100⌉).

Le code externe est un **LDPC systématique**, dont la matrice de contrôle se reconstruit d'une
graine fixe (`0x4C445043`), à l'identique chez l'encodeur et chez le décodeur :

- le mot de code compte *k* = d · 104 bits d'information puis *m* = ⌈d · p / 100⌉ · 104 bits de
  parité ;
- chaque bit d'information participe à quatre contrôles, tirés au hasard (flux `0x13`) sans
  cycle de longueur quatre — deux variables aux mêmes deux contrôles, un bit de parité
  comptant pour ses contrôles —, en chargeant de préférence les contrôles les moins remplis ;
- le bit de parité *j* participe aux contrôles *j* et *j + 1* ; le premier contrôle n'a que le
  bit de parité 0.

L'encodage est une accumulation : la parité *j* vaut la parité *j − 1*, plus la somme modulo 2
des bits d'information du contrôle *j*. Le décodage est une propagation de croyances, min-sum
normalisé d'un facteur 0,8, en au plus 60 allers-retours entre variables et contrôles ; elle
s'arrête dès que chaque contrôle est satisfait.

### Entrelacement

Dans le mot de code d'un bloc, le bit de rang *b* du brin de données *s* occupe le rang
*b* · d + *s* ; celui du brin de parité *s*, le rang k + *b* · m/104 + *s*. Les bits d'un même
rang passent donc consécutivement, et ceux d'un même brin se trouvent à *d* (ou *m*/104)
positions les uns des autres : un brin manquant, salve de 104 effacements, devient 104
effacements isolés.

### Lecture d'un brin

1. Chaque base lue pèse ses deux bits par un rapport de vraisemblance de +4 ou −4.
2. Le décodage de Viterbi donne le chemin le plus probable. Si ce chemin redonne la lecture à
   l'identique, la lecture est un mot de code : ses rapports a posteriori dépassent tous la borne,
   et valent donc, bornés, +4 ou −4 selon chaque bit. Sinon, l'algorithme log-MAP (BCJR) parcourt
   le treillis dans les deux sens et rend le rapport a posteriori de chacun des 148 bits
   d'information, borné à ±4.
3. Les 20 premiers bits donnent le sel, qui démasque l'index. Une lecture d'index hors de
   l'archive est écartée.
4. Les 104 bits de charge utile sont démasqués : leur rapport change de signe où le flux
   vaut 1.
5. Les lectures d'un même brin s'additionnent. Un brin sans lecture garde des rapports nuls :
   ce sont des effacements.
6. Bloc par bloc, la propagation de croyances tranche. Un bloc où manquent plus de brins
   que la parité n'en porte est refusé sans décodage ; de même un bloc qui n'a pas convergé.

Les rapports sont pris à une échelle fixe, qui n'importe pas : le min-sum est invariant par
homothétie, et les décisions du treillis aussi.

### Paramètres

| Paramètre | Défaut | Rôle |
|-----------|--------|------|
| `parity`  | `20`   | brins de parité pour cent brins de données, de 5 à 100 |

Le manifeste porte la parité au premier octet de `c`, où goldman range son identifiant de
fichier. Ses oligos comptent deux octets de parité Reed-Solomon, soit 152 nt.

### Taille maximale

L'index tient sur 2²⁴ brins, données et parité : 13 981 013 brins de données à la parité de
20 %, soit environ 173 Mio.

## 11. Fichiers FASTA et FASTQ

Une archive s'écrit dans un fichier FASTA ordinaire, une séquence repliée à 60 nucléotides par
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
- `<schéma>` vaut `fountain`, `strands`, `goldman` ou `grass`.

La lecture tolère les fins de ligne CRLF, les lignes repliées, les minuscules, les lignes vides
et les lignes de commentaire commençant par `;`. Elle refuse les chiffres et la ponctuation
dans les séquences. Les codes d'ambiguïté tels que `N` sont acceptés à la lecture ; les décodeurs
les écartent et les comptent.

Plusieurs fichiers de lectures peuvent être donnés au décodage : leurs séquences sont
concaténées.

### FASTQ

Les lectures d'un séquenceur arrivent en FASTQ. Le lecteur le reconnaît à son premier
caractère utile, un `@`, et lit par blocs de quatre lignes : en-tête, séquence, ligne `+`,
qualités. La ligne de qualités doit compter un caractère par base ; elle est contrôlée puis
ignorée, les décodeurs ne s'en servent pas. Les mêmes tolérances s'appliquent aux séquences
(CRLF, minuscules, blancs) ; un enregistrement incomplet, un en-tête vide ou une ligne `+`
manquante est refusé, avec son numéro de ligne.

## 12. Décodage

1. **Normalisation.** Majuscules et blancs tolérés ; tout autre caractère écarte la lecture.
2. **Manifeste.** Il donne le schéma et les paramètres de l'archive
   ([section 5](#5-manifeste)).
3. **Corps**, selon le schéma ([6](#6-schéma-dna-fountain), [7](#7-schéma-brins-codés),
   [8](#8-schéma-goldman), [9](#9-schéma-grass) ou [10](#10-schéma-ldpc)).
4. **Enveloppe.** CRC-32, décompression, puis comparaison de l'empreinte SHA-256.

Un schéma attendu peut être imposé ; une archive d'un autre schéma est alors refusée.

## 13. Générateur pseudo-aléatoire

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
| `0x14` | masque des symboles d'un brin grass (nonce, charge utile, parité) |

**`scramble_u32`** disperse un compteur sur 32 bits, pour les graines, les nonces de manifeste
et le choix des bases de repère des brins goldman :

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

## 14. Limites récapitulatives

| | DNA Fountain | Brins codés | goldman | grass | ldpc |
|---|---|---|---|---|---|
| Oligo | 152 nt (par défaut) | 160 nt | 117 nt | 117 nt | 154 nt |
| Corps maximal | ≈ 137 Gio | ≈ 420 Mio | ≈ 2,6 Mo | ≈ 55 Mo (parité 14) | ≈ 173 Mio (parité 20) |
| Sans ordre | oui | non : les brins sont adressés | non : les brins sont adressés | non : les brins sont adressés | non : les brins sont adressés |
| Perte aléatoire supportée | `1 − 1/r` des oligos, par construction (`r` = marge) | 32 brins manquants sur 255, soit 12,5 % d'un bloc | chaque base doit rester dans l'un de ses quatre brins : environ 2 % de brins perdus | 92 brins manquants sur 713, soit 13 % d'un bloc | 11 % des brins (parité 20), 25 % (parité 50) |
| Substitutions | dépend de la parité : avec 2 octets, une seule erreur par lecture est corrigée | 2 octets erronés par brin | quatre lectures par base, sans code correcteur | 7 symboles (21 nt) erronés par lecture, avec la parité par défaut | 5 % par lecture, à couverture 10 |
| GC | 45 à 55 % garanti | 45 à 55 % garanti | non garanti (50 % en moyenne) | 45 à 55 % garanti | 45 à 55 % garanti |
| Limite pratique | vitesse d'encodage | taille | taille ; couverture nécessaire | densité | vitesse de décodage ; couverture de 3 au moins |

## 15. Version 1

Les archives produites par la version 0.1 ne sont plus lues : leurs en-têtes portent la marque
`|scheme:`, et les schémas qu'elles annonçaient ne mettaient pas en œuvre ce que leur nom
promettait. Les noms `adaptive`, `base3` et `ultimate` sont refusés avec un message explicite.
Les noms `goldman` et `grass` désignent désormais les schémas des sections 8 et 9, et
`goldman2013` et `grass2015` en sont des alias. Pour relire une archive de la version 0.1,
utiliser le code du commit `09df4f8`.

## 16. Pool de bibliothèque

Hors du format d'archive : un pool de molécules pour séquenceur, écrit par `adn library`,
est un FASTA à un enregistrement par molécule. Son en-tête dit l'échantillon et ses index :

```
>sample=<nom> i7=<nom>:<bases> i5=<nom>:<bases>
```

L'index i5 est facultatif. La séquence est la molécule complète, au format Nextera. Ces
en-têtes sont lus par `adn simulate --library` et `adn demux --library` ; ils ne servent pas à
décoder une archive. Le détail est dans [`LIBRARY.md`](LIBRARY.md).

## Références

- Viterbi A. — *Error bounds for convolutional codes and an asymptotically optimum decoding
  algorithm*, IEEE Transactions on Information Theory 13(2):260–269, 1967.
- Bahl L. R., Cocke J., Jelinek F., Raviv J. — *Optimal decoding of linear codes for minimizing
  symbol error rate*, IEEE Transactions on Information Theory 20(2):284–287, 1974.
- Gallager R. G. — *Low-density parity-check codes*, MIT Press, 1963.
- MacKay D. J. C., Neal R. M. — *Near Shannon limit performance of low density parity check
  codes*, Electronics Letters 33(6):457–458, 1997.

- Erlich Y., Zielinski D. — *DNA Fountain enables a robust and efficient storage architecture*,
  Science 355(6328):950–952, 2017.
- Luby M. — *LT codes*, FOCS 2002.
- Goldman N., Bertone P., Chen S., Dessimoz C., Leong E. M., Sipos B., Birney E. — *Towards
  practical, high-capacity, low-maintenance information storage in synthesised DNA*, Nature
  494:77–80, 2013 ; et son information complémentaire, *Method for Encoding and Decoding
  Arbitrary Computer Files in DNA Fragments*.
- Grass R. N., Heckel R., Puddu M., Paunescu D., Stark W. J. — *Robust Chemical Preservation of
  Digital Information on DNA in Silica with Error-Correcting Codes*, Angewandte Chemie
  International Edition 54:2552–2555, 2015.
- Gao S. — *A new algorithm for decoding Reed-Solomon codes*, Communications, Information and
  Network Security, 2003 (actes de CISS 2002).
