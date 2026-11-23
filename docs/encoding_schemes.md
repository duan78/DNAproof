# Les schémas d'encodage

Quatre schémas répartissent un fichier en oligos. Ils partagent la même enveloppe, les mêmes
manifestes et les mêmes contraintes biochimiques ([format](FORMAT.md)) ; ils diffèrent par la
façon dont les oligos portent les données, donc par ce qu'ils savent traverser.

- **`fountain`** — DNA Fountain (Erlich et Zielinski, 2017) : des gouttes sans ordre ni adresse.
- **`strands`** — brins codés : des brins adressés, protégés par deux niveaux de Reed-Solomon.
- **`goldman`** — Goldman et al., 2013 : un trit par base, jamais deux bases identiques de
  suite, chaque base lue dans quatre brins recouvrants.
- **`grass`** — Grass et al., 2015 : Reed-Solomon sur le corps à 47 éléments, un symbole par
  groupe de trois bases.

Le schéma se choisit à l'encodage. Il est écrit dans le manifeste de l'archive, et le décodage le
lit : il n'y a jamais à s'en souvenir.

## DNA Fountain

### Principe

Le corps du fichier est coupé en blocs de 32 octets. Une *goutte* est le OU exclusif de quelques
blocs, désignés par une graine de 4 octets : la graine donne le nombre de blocs (degré, tiré d'une
distribution soliton robuste) et leurs rangs. L'oligo porte la graine, la goutte, et deux octets
de parité : 38 octets, soit 152 nt.

Un oligo ne porte ni adresse ni rang. L'archive se reconstitue par *épluchage* : une goutte qui ne
mélange plus qu'un bloc inconnu le révèle ; ce bloc est retiré des autres gouttes qui le
mélangent, et ainsi de suite. Les lectures arrivent dans n'importe quel ordre, et n'importe
lesquelles manquent : seules comptent les gouttes distinctes.

### Ce que règle l'encodeur

| Paramètre | Défaut | Effet |
|-----------|--------|-------|
| marge de perte (`--redundancy`) | `1.1` | `r` : l'archive reste décodable après la perte aléatoire d'une fraction `1 − 1/r` des oligos, contrôlé sur 256 tirages |
| parité par oligo (`--rs-parity`) | `2` | un oligo résiste à la moitié d'octets erronés ; l'oligo s'allonge de 4 nt par octet |
| `c`, `δ` de la distribution | automatique, `0.001` | ajustés au nombre de blocs |

**La marge n'est pas un chiffre indicatif.** L'encodeur tient un décodeur à jour et n'arrête
d'ajouter des gouttes que lorsque l'archive reste décodable après l'écart aléatoire de la fraction
`1 − 1/r` des oligos, pour chacun de 256 tirages indépendants. Un code LT a une queue de
distribution lourde : sans ce contrôle, une archive à peine décodable échoue une fois sur dix dès
qu'on lui retire des oligos.

**Les petits fichiers coûtent cher.** Pour quelques dizaines de blocs, la distribution soliton
réclame beaucoup plus d'une goutte par bloc : un fichier de 75 octets donne 141 oligos, un de
4 Ko environ 2,9 gouttes par bloc, et une densité de 0,5 bit par nucléotide. Au-delà de quelques
centaines de kilooctets, on approche 1,2 goutte par bloc.

**La parité par oligo.** Avec deux octets, une lecture ne corrige qu'un octet erroné. Les erreurs
de *synthèse* — celles qui sont dans l'oligo lui-même, et que toutes ses lectures portent à
l'identique — ne peuvent être démasquées par aucun vote : c'est la parité qui les corrige, ou rien.
Huit octets font passer une synthèse à 1 % de substitution.

## Brins codés

### Principe

Le corps est coupé en tranches de 30 octets, chacune dans un brin avec son index (3 octets) et
quatre octets de parité interne : le mot de code de 37 octets est masqué par un flux dérivé d'un
nonce de 3 octets. Un brin fait 40 octets, soit 160 nt.

- **Code interne** : quatre octets de parité par brin corrigent jusqu'à 2 octets erronés par
  lecture.
- **Code externe** : les brins sont groupés par blocs de 223, et chaque bloc reçoit 32 brins de
  parité, colonne par colonne. Le code corrige, dans chaque bloc, tout motif tel que
  `2 · erreurs + brins manquants ≤ 32` : 32 brins manquants, ou 16 brins faux.

### Ce qu'il faut savoir

- Les brins sont adressés : l'archive a une taille fixe, sans redondance réglable.
- L'index tient sur 24 bits : le corps ne dépasse pas 420 Mio.
- C'est le schéma le plus économe pour les petits fichiers : 49 brins pour 75 octets.

## Goldman et al., 2013

### Principe

Le schéma de l'article, suivi étape par étape.

1. Les octets deviennent des trits (0, 1, 2) par un code de Huffman ternaire : les octets les
   plus fréquents du fichier s'écrivent en moins de trits. Le code est construit pour chaque
   archive et porté par elle ; quand il ne rapporte pas — fichier incompressible —, un code fixe
   de 5 ou 6 trits prend le relais.
2. Chaque trit choisit l'une des trois bases différentes de la précédente : jamais deux bases
   identiques de suite. Une répétition signale donc une erreur de lecture.
3. L'ADN est relu par fenêtres de 100 nt décalées de 25 : chaque base du message apparaît dans
   quatre brins. Un brin fait 117 nt : une base de repère, 100 nt de message, un index de 15 nt
   (identifiant de fichier, adresse, trit de parité), une seconde base de repère.
4. Au décodage, chaque base du message est votée par les brins qui la couvrent ; la base qui
   rassemble le plus de voix l'emporte.

### Ce qu'il faut savoir

- **Le GC n'est pas garanti par brin.** La conversion des trits en bases est une bijection, sans
  marge de manœuvre : comme dans l'article, le GC est de 50 % en moyenne, sans borne par brin.
  L'encodage n'admet que les bornes de GC par défaut, ou des bornes ouvertes.
- **La couverture est nécessaire.** Il n'y a pas de code correcteur : une erreur dans l'index
  d'un brin (ses quinze dernières bases) le fait perdre, et une base n'est restituée que si l'un
  de ses quatre brins a été lu. Comptez une couverture de 5.
- **La place est chère.** Le recouvrement quadruple met la densité à 0,34 bit par nucléotide, et
  l'adresse à 12 trits limite le corps à 2,6 Mo.
- **Plusieurs archives dans un tube.** `--file-id <0-8>` inscrit un identifiant dans chaque
  index : on mélange les archives, et le décodage n'en relit qu'une.
- **Les erreurs de synthèse lui font mal.** Elles sont dans l'oligo, et toutes ses lectures les
  portent : dans l'index, elles ne se compensent par aucune couverture.

## Grass et al., 2015

### Principe

L'article encode l'information en caractères du corps à 47 éléments, corrigés par du
Reed-Solomon, et écrit chaque caractère en trois bases. Le corps à 47 éléments vient de là : il y
a 36 trimères sans deux bases adjacentes égales et 12 trimères terminés par un doublet, et 47 est
le plus grand premier de cette taille. Les trimères retenus (47 sur 64) ne forment jamais de suite
de plus de trois bases identiques ; un trimère hors du jeu signale une erreur à sa position, que
le décodeur traite comme un effacement.

Un brin fait 39 symboles, soit 117 nt : un index de 4 symboles, un nonce, une charge utile, et
une parité interne. Deux niveaux de code :

- **Code interne** : Reed-Solomon sur GF(47), brin par brin. Avec la parité par défaut (14
  symboles), une lecture résiste à sept symboles erronés, soit 21 bases — ainsi qu'à quatorze
  effacements.
- **Code externe** : un Reed-Solomon sur GF(47²) par bloc de 713 brins, le bloc de l'article —
  92 brins de parité pour 621 de données. Il répare jusqu'à 92 brins manquants par bloc, soit
  13 %, ou corrige 46 brins faux.

L'adressage logique et le code externe sont des ajouts du format v2 : l'article adressait ses
brins par les amorces. Ses paramètres internes n'ont pas pu être vérifiés (information
complémentaire non accessible) ; le découpage 14 symboles de parité sur 39, soit 25 de donnée,
est celui qui rend compte des chiffres de l'article (83 Ko dans 4 991 brins).

### Ce qu'il faut savoir

- C'est le schéma qui traverse le plus de substitutions : 5 % par nucléotide à couverture 3.
- Il traverse 10 % de perte à couverture 5.
- La parité interne se règle (`--rs-parity`, nombre pair de 2 à 30) : moins de parité, plus de
  charge utile et moins de résistance. À 6, un brin porte 19 octets au lieu de 13, et la densité
  passe de 0,77 à 1,12 bit par nucléotide.
- Le bloc de 713 brins coûte cher aux petits fichiers : un fichier de 75 octets donne 103 brins,
  dont 92 de parité externe. Pour les petits fichiers, `strands` reste l'économique (49 brins).

## Choisir

| Vous avez… | Prenez |
|------------|--------|
| un petit fichier (moins de quelques centaines de kilooctets) | `strands` |
| un fichier de plus de 420 Mio | `fountain` |
| un canal qui perd des oligos | `fountain`, en relevant la marge (`--redundancy 1.5` voire plus) |
| un canal à substitutions, peu de pertes | `strands` : il traverse 2 % de substitution |
| un canal à substitutions denses | `grass` : il traverse 5 % de substitution à couverture 3 |
| un séquençage à insertions et suppressions | `fountain` avec une marge large et beaucoup de couverture, ou un séquençage meilleur : les lectures qui changent de longueur sont perdues, par les quatre schémas |
| une synthèse bruitée | `fountain` avec `--rs-parity 8`, ou `grass` |
| les schémas des articles, tels qu'ils sont publiés | `goldman` et `grass`, avec leurs limites propres |

## Tolérance aux erreurs

Mesurée avec `adn simulate`, sur un fichier de 1 Mio d'octets incompressibles, 10 tirages par
case. « 10/10 » : le fichier est restitué dans les dix tirages. **Sur les mille tirages des deux
tableaux, aucun fichier faux n'a jamais été rendu** : un échec est toujours annoncé.

La *couverture* est le nombre **moyen** de lectures par oligo, tiré d'une loi de Poisson — comme
au séquençage. À couverture 1, 37 % des oligos n'ont aucune lecture : aucun schéma ne s'en
remet. Les taux sont par nucléotide.

| Scénario | `fountain` marge 1,1 | `fountain` marge 1,5 | `strands` |
|----------|:--------------------:|:--------------------:|:---------:|
| substitutions 0,1 %, couverture 1 | 0/10 | 0/10 | 0/10 |
| substitutions 0,1 %, couverture 3 | 10/10 | 10/10 | 10/10 |
| substitutions 0,1 %, couverture 5 | 10/10 | 10/10 | 10/10 |
| substitutions 0,1 %, couverture 10 | 10/10 | 10/10 | 10/10 |
| substitutions 0,5 %, couverture 5 | 6/10 | 10/10 | 10/10 |
| substitutions 0,5 %, couverture 10 | 10/10 | 10/10 | 10/10 |
| substitutions 1 %, couverture 10 | 0/10 | 2/10 | 10/10 |
| substitutions 1 %, couverture 20 | 10/10 | 10/10 | 10/10 |
| substitutions 1 %, couverture 30 | 10/10 | 10/10 | 10/10 |
| substitutions 2 %, couverture 50 | 0/10 | 10/10 | 10/10 |
| insertions et suppressions 0,5 %, couverture 5 | 0/10 | 7/10 | 0/10 |
| insertions et suppressions 0,5 %, couverture 10 | 6/10 | 6/10 | 0/10 |
| perte de 5 % des oligos | 10/10 | 10/10 | 10/10 |
| perte de 8 % des oligos | 10/10 | 10/10 | 10/10 |
| perte de 10 % des oligos | 10/10 | 10/10 | 0/10 |
| perte de 15 % des oligos | 0/10 | 10/10 | 0/10 |
| perte de 25 % des oligos | 0/10 | 10/10 | 0/10 |
| synthèse : substitutions 0,5 %, couverture 5 | 0/10 | 0/10 | 0/10 |
| tout combiné¹, couverture 10 | 0/10 | 10/10 | 10/10 |

¹ substitutions 0,2 %, insertions 0,1 %, suppressions 0,1 %, synthèse 0,1 %, perte de 5 %.

### Lire ce tableau

- **Substitutions.** `strands` corrige deux octets par brin : il traverse 2 % de substitution.
  `fountain` n'en corrige qu'un ; il compte alors sur les lectures reçues sans erreur, et il en
  faut plusieurs par oligo dès que le taux dépasse 0,1 % : à 1 %, vingt lectures par oligo,
  quelle que soit la marge. À 2 %, seule la marge de 1,5 s'en tire, avec cinquante lectures.
- **Perte.** `fountain` traverse ce que sa marge promet, et un peu plus : à 1,1, 10 % de perte.
  `strands` répare 32 brins manquants sur 255 par bloc — 12,5 % en théorie, 8 % en pratique avec
  une couverture de 5, car les blocs les plus malmenés décident du sort de l'archive.
- **Insertions et suppressions.** Une lecture dont la longueur a changé est écartée, par les deux
  schémas : le décodage travaille sur des lectures de longueur exacte. À 0,5 % de chaque, il
  reste environ 20 % de lectures utilisables. `fountain` s'en tire le mieux — 7 tirages sur 10,
  à marge 1,5 — ; `strands` jamais.
- **Synthèse.** Une erreur de synthèse est dans l'oligo : toutes ses lectures la portent. Avec
  deux octets de parité, trois erreurs dans un oligo suffisent à perdre son avis, et aucun vote ne
  peut le rattraper.

### La parité par oligo, devant les erreurs de synthèse

`fountain` à marge 1,1, couverture 5 sauf mention contraire :

| Scénario | parité 4 (160 nt) | parité 8 (176 nt) |
|----------|:-----------------:|:-----------------:|
| synthèse : substitutions 0,5 % | 0/10 | 10/10 |
| synthèse : substitutions 1 % | 0/10 | 10/10 |
| synthèse 1 % et substitutions 1 %, couverture 10 | 0/10 | 5/10 |
| substitutions 2 %, couverture 10 | 0/10 | 2/10 |
| substitutions 5 %, couverture 30 | 0/10 | 0/10 |

La parité protège des erreurs que toutes les lectures partagent ; elle ne remplace pas la
couverture contre celles que chaque lecture porte seule.

### Les schémas des articles

Mêmes conditions, sur un fichier de 256 Kio d'octets incompressibles (le schéma goldman est limité
à 2,6 Mo). Un tiret : le scénario ne s'applique pas à la colonne.

| Scénario | `goldman` | `grass` parité 14 | `grass` parité 6 |
|----------|:---------:|:-----------------:|:----------------:|
| couverture 3 seule | — | 10/10 | — |
| couverture 5 seule | 10/10 | — | — |
| couverture 10 seule | 10/10 | — | — |
| substitutions 0,1 %, couverture 5 | 10/10 | — | — |
| substitutions 0,5 %, couverture 3 | — | 10/10 | 10/10 |
| substitutions 0,5 %, couverture 5 | 10/10 | — | — |
| substitutions 1 %, couverture 3 | — | 10/10 | 10/10 |
| substitutions 1 %, couverture 10 | 10/10 | — | — |
| substitutions 2 %, couverture 3 | — | 10/10 | 0/10 |
| substitutions 2 %, couverture 20 | 10/10 | — | — |
| substitutions 3 %, couverture 3 | — | 10/10 | — |
| substitutions 5 %, couverture 3 | — | 10/10 | 0/10 |
| insertions et suppressions 0,5 %, couverture 5 | — | 0/10 | — |
| insertions et suppressions 1 %, couverture 10 | 0/10 | — | — |
| perte 1 %, couverture 10 | 10/10 | — | — |
| perte 2 %, couverture 10 | 10/10 | — | — |
| perte 5 %, couverture 5 | — | 10/10 | — |
| perte 5 %, couverture 10 | 10/10 | — | — |
| perte 8 %, couverture 5 | — | 10/10 | 10/10 |
| perte 10 %, couverture 5 | — | 10/10 | — |
| perte 12 %, couverture 5 | — | 0/10 | — |
| synthèse : substitutions 1 %, couverture 3 | — | 10/10 | 9/10 |
| synthèse : substitutions 1 %, couverture 10 | 0/10 | — | — |
| synthèse : substitutions 5 %, couverture 3 | — | 0/10 | — |
| synthèse : substitutions 5 %, couverture 10 | 0/10 | — | — |

- **`grass` est le plus résistant aux substitutions** : 5 % par nucléotide à couverture 3, grâce
  à sept symboles corrigés par lecture — et un trimère hors du jeu des 47 est un effacement, qui
  ne compte pas dans les erreurs. Avec une parité de 6, il tombe à trois symboles : 1 % passe,
  2 % plus.
- **`grass` traverse 10 % de perte** à couverture 5 : 92 brins manquants sur 713 par bloc.
- **`goldman` a besoin de couverture**, faute de code correcteur : 5 au moins, parce qu'une base
  n'est restituée que si l'un de ses quatre brins a été lu, et qu'à couverture 3, 5 % des brins
  manquent. À couverture 10, il traverse 5 % de perte et 2 % de substitution.
- **Les erreurs de synthèse font perdre `goldman`** : à 1 %, aucune récupération. Une erreur dans
  l'index d'un brin le fait perdre, et toutes ses lectures portent la même erreur : aucune
  couverture ne le rattrape. C'est la faiblesse du schéma publié, qui comptait sur une synthèse
  très propre.
- **Les insertions et suppressions perdent les deux** comme les autres : une lecture de 117 nt
  qui change de longueur est écartée, et il en reste environ un tiers à 0,5 % de chaque.

## Refaire ces mesures

```sh
adn encode --input fichier.bin --output archive.fasta
adn simulate --archive --input archive.fasta \
    --substitution-rate 0.01 --coverage 20 --trials 10
```

Sans `--archive`, `simulate` encode le fichier avec les options données, puis le dégrade.
`--synthesis-substitution-rate`, `--insertion-rate`, `--deletion-rate` et `--dropout` règlent le
canal ; `--seed` rend un tirage reproductible.

## Références

- Erlich Y., Zielinski D. — *DNA Fountain enables a robust and efficient storage architecture*,
  Science 355(6328):950–952, 2017.
- Goldman N., Bertone P., Chen S., Dessimoz C., Leong E. M., Sipos B., Birney E. — *Towards
  practical, high-capacity, low-maintenance information storage in synthesised DNA*, Nature
  494:77–80, 2013.
- Grass R. N., Heckel R., Puddu M., Paunescu D., Stark W. J. — *Robust Chemical Preservation of
  Digital Information on DNA in Silica with Error-Correcting Codes*, Angewandte Chemie
  International Edition 54:2552–2555, 2015.
- Luby M. — *LT codes*, FOCS 2002.
