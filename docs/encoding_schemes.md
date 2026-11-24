# Les schémas d'encodage

Six schémas répartissent un fichier en oligos. Ils partagent la même enveloppe, les mêmes
manifestes et les mêmes contraintes biochimiques ([format](FORMAT.md)) ; ils diffèrent par la
façon dont les oligos portent les données, donc par ce qu'ils savent traverser.

- **`fountain`** — DNA Fountain (Erlich et Zielinski, 2017) : des gouttes sans ordre ni adresse.
- **`strands`** — brins codés : des brins adressés, protégés par deux niveaux de Reed-Solomon.
- **`goldman`** — Goldman et al., 2013 : un trit par base, jamais deux bases identiques de
  suite, chaque base lue dans quatre brins recouvrants.
- **`grass`** — Grass et al., 2015 : Reed-Solomon sur le corps à 47 éléments, un symbole par
  groupe de trois bases.
- **`ldpc`** — code convolutif par brin et LDPC par bloc, à décisions souples, entrelacés.
- **`hedges`** — code à hachis par brin (HEDGES), décodé par une recherche : le seul qui
  corrige les insertions et les suppressions.

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
Huit octets font passer une synthèse à 1 % de substitution, neuf fois sur dix.

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
- Il traverse 8 % de perte à couverture 5.
- La parité interne se règle (`--rs-parity`, nombre pair de 2 à 30) : moins de parité, plus de
  charge utile et moins de résistance. À 6, un brin porte 19 octets au lieu de 13, et la densité
  passe de 0,77 à 1,12 bit par nucléotide.
- Le bloc de 713 brins coûte cher aux petits fichiers : un fichier de 75 octets donne 103 brins,
  dont 92 de parité externe. Pour les petits fichiers, `strands` reste l'économique (49 brins).

## Code convolutif et LDPC

### Principe

Les quatre premiers schémas décident base à base, puis corrigent des octets ou des symboles
faux. `ldpc`, lui, pèse chaque bit : un rapport de vraisemblance, `ln P(bit = 0) / P(bit = 1)`, nul
pour un effacement. Deux codes en série échangent ces rapports :

- **Code interne**, brin par brin : le code convolutif binaire (171, 133) en octal, longueur de
  contrainte 7, rendement ½ — celui des normes de communication spatiale. Chaque bit
  d'information donne deux bits de code, soit une base. Décodé par l'algorithme log-MAP, il
  rend un rapport par bit, pas un verdict : une lecture à 5 % d'erreurs pèse encore.
- **Entrelacement** : dans le mot de code d'un bloc, les bits de même rang passent
  consécutivement. Un brin sans lecture est une salve de 104 effacements ; l'entrelacement la
  disperse en effacements isolés.
- **Code externe**, bloc par bloc : un LDPC de 819 brins de données, dont la matrice se
  reconstruit d'une graine, décodé par propagation de croyances. Il répare les brins manquants
  et les bits faux qui restent.

Un brin fait 154 nt : un sel de 20 bits, un index de 24, une charge utile de 13 octets, et
six zéros de queue. Le sel règle le masquage de tout le brin, et varie à l'encodage jusqu'à ce
que le brin respecte les contraintes biochimiques.

### Ce qu'il faut savoir

- C'est le schéma qui traverse le plus d'erreurs dispersées : 5 % de substitution par
  nucléotide à couverture 10, et 6 % à couverture 20 — à 7 %, quatre tirages sur dix. `grass`
  s'arrête à 5 %, à couverture 3.
- C'est le seul qui traverse les erreurs de synthèse au-delà de 1 % : 5 % à couverture 10, une
  fois sur deux.
  Elles frappent l'ADN lui-même, et le code convolutif les corrige comme des erreurs de
  lecture : plusieurs lectures les confirment, au lieu de les voter.
- Il demande de la couverture : 2,5 lectures par brin en moyenne au moins. À couverture 2,
  13 % des brins manquent, et la parité par défaut n'en répare que 11 %.
- La parité se règle (`--rs-parity`, de 5 à 100 pour cent) : à 50 %, 22 % de perte — sept
  tirages sur dix à 25 % —, mais la densité tombe de 0,56 à 0,45 bit par nucléotide.
- C'est le plus lent à décoder, de loin : chaque lecture passe sur un treillis de 64 états.
- Comme les autres, il perd la plupart des lectures dont la longueur a changé : à 0,5 %
  d'insertions et de suppressions, quatre tirages sur dix seulement. `hedges` est le seul à
  les lire.

## HEDGES

### Principe

Les cinq premiers schémas lisent chaque lecture à sa place : une insertion ou une suppression
décale tout ce qui suit, et la lecture est perdue. `hedges` rétablit la synchronisation. Il met en
œuvre HEDGES (Press et al., 2020), un code « arbre » à longueur de contrainte infinie.

- **Code interne**, brin par brin : chaque base porte zéro, un ou deux bits d'un message, selon
  un motif répété. La base émise est désignée par un hachis de la position, des derniers bits du
  message et du sel — l'index du brin. Le message ne se lit donc pas dans l'ADN : il se déduit
  du hachis, qui dépend de tout ce qui précède. Une erreur de lecture, ou de position, dérègle
  toutes les prédictions qui suivent — et le décodeur s'en aperçoit.
- **Lecture** : une recherche best-first, un décodage séquentiel à pile. Une hypothèse devine
  les bits de la position courante, sa place dans le message et sa place dans la lecture ; la
  base qui correspond rapporte une récompense, chaque erreur coûte un point ; l'hypothèse la
  moins chère est développée la première. Une lecture sans défaut ne coûte que ses
  développements, un par base du brin.
- **Code externe**, bloc par bloc : un Reed-Solomon (255, 223) appliqué « en diagonale » — le mot
  de code *j* prend, du brin *s*, l'octet de rang (j + s) mod L. Il restitue 32 brins manquants
  par bloc, ou en corrige 16 de faux ; les lectures d'un même brin votent, et à égalité le brin
  est écarté.

Un brin fait 200 ou 300 nt selon le rendement, c'est-à-dire selon le motif de bits par base :
tous tiennent dans une lecture de 300 bases.

| rendement | motif | brin | charge utile | densité | erreurs traversées par base, des trois sortes réunies |
|-----------|-------|------|--------------|---------|------------------------------------------------------|
| ¾         | 2, 1    | 200 nt | 32 octets | 1,12 bit/nt | 1,5 % à couverture 10 ; 3 % à couverture 20 |
| ½         | 1       | 200 nt | 20 octets | 0,70 bit/nt | 4,5 % à couverture 10 ; 9 % à couverture 20 |
| ⅓         | 1, 1, 0 | 300 nt | 20 octets | 0,47 bit/nt | 6 % à couverture 10 ; 12 % à couverture 20 |
| ¼         | 1, 0    | 300 nt | 13 octets | 0,30 bit/nt | 15 % à couverture 10 ; 18 % à couverture 20 |

La densité est mesurée sur 1 Mio d'octets au hasard, manifestes compris : 1,08, 0,69, 0,46 et
0,30 bit/nt sur 64 Kio.

### Ce qu'il faut savoir

- **C'est le seul schéma qui corrige les insertions et les suppressions.** À 1,5 % d'erreurs
  des trois sortes réunies, à couverture 10, `fountain`, `strands`, `goldman` et `ldpc` échouent
  à chaque tirage ; `grass` s'en tire, parce qu'il lui reste assez de lectures de longueur
  exacte.
- **Le rendement est le seul réglage** (`--code-rate`) : moins le code porte, plus il corrige —
  à couverture 10, 1,5 % d'erreurs des trois sortes réunies par base au ¾, 4,5 % au ½, 6 % au
  ⅓, 15 % au ¼.
- **Les contraintes sont dans le code**, sans criblage : chaque oligo respecte de 45 à 55 % de GC
  et trois bases identiques au plus. Le code ne garantit rien de plus étroit : des bornes plus
  serrées, ou deux bases au plus, sont refusées.
- **La perte n'est pas réglable** : 7 % passent, 8 % ne passent que neuf fois sur dix. Pour un
  canal qui perd des oligos, `fountain` et sa marge, ou `ldpc` et sa parité.
- **Une lecture qui résiste coûte cher** : elle va au bout de son budget, 65 536 hypothèses,
  de 3,8 à 4,7 ms — là où une lecture intacte en prend 14 à 20 µs. C'est le prix d'une lecture
  d'une autre archive, rangée dans la mauvaise par un saut d'index, et de toute lecture par
  delà ce que le rendement corrige : à ¾, une lecture sur huit à 1,5 % d'erreurs.
- **Au-delà de ce que le code corrige, le vrai chemin n'est plus toujours le moins cher.**
  Pris isolément, à ¼, 1,9 lecture sur mille rend un message faux à 10 % d'erreurs, 3,8 à
  12 %. Le vote des lectures du brin, le code externe et l'empreinte du fichier l'arrêtent : sur
  les 2 750 tirages des tableaux, aucun fichier faux.

## Choisir

| Vous avez… | Prenez |
|------------|--------|
| un petit fichier (moins de quelques centaines de kilooctets) | `strands` |
| un fichier de plus de 420 Mio | `fountain` |
| un canal qui perd des oligos | `fountain`, en relevant la marge (`--redundancy 1.5` voire plus) |
| un canal à substitutions, peu de pertes | `strands` : il traverse 2 % de substitution |
| un canal à substitutions denses | `grass` : il traverse 5 % de substitution à couverture 3 ; `ldpc` va plus loin, à couverture 10 |
| un séquençage à insertions et suppressions | `hedges` : 4,5 % d'erreurs des trois sortes réunies par base à couverture 10 (rendement ½), 15 % (rendement ¼). Les autres y perdent les lectures qui changent de longueur |
| un canal très bruité, quel que soit le genre d'erreur | `hedges` au rendement ¼ : 15 % d'erreurs des trois sortes réunies à couverture 10, 18 % à couverture 20 ; `ldpc` s'arrête à 6 % de substitutions, et ne lit pas celles qui changent la longueur |
| une synthèse bruitée | `ldpc` : 5 % d'erreurs de synthèse à couverture 10 ; `hedges` lit aussi celles qui changent la longueur (3 % à ½) ; sinon `fountain` avec `--rs-parity 8`, ou `grass` |
| les schémas des articles, tels qu'ils sont publiés | `goldman` et `grass`, avec leurs limites propres |

## Tolérance aux erreurs

Mesurée avec `adn simulate`, sur un fichier de 1 Mio d'octets incompressibles, 10 tirages par
case. « 10/10 » : le fichier est restitué dans les dix tirages. **Sur les 2 750 tirages de ces
tableaux, aucun fichier faux n'a jamais été rendu** : un échec est toujours annoncé.

La *couverture* est le nombre **moyen** de lectures par oligo, tiré d'une loi de Poisson — comme
au séquençage. À couverture 1, 37 % des oligos n'ont aucune lecture : aucun schéma ne s'en
remet. Les taux sont par nucléotide.

Les dix tirages d'une case sont indépendants. Ils ne l'étaient pas tout à fait : jusqu'à la
correction du simulateur (voir le [journal des changements](../CHANGELOG.md)), des graines
voisines ne faisaient que déplacer les oligos sans lecture, et devant la perte ou une faible
couverture, une case mesurait un seul tirage. Toutes les matrices ont été reprises : un cas
limite y montre sa vraie probabilité — 4/10, 7/10 — au lieu d'un 10/10 ou d'un 0/10 de
circonstance.

| Scénario | `fountain` marge 1,1 | `fountain` marge 1,5 | `strands` |
|----------|:--------------------:|:--------------------:|:---------:|
| substitutions 0,1 %, couverture 1 | 0/10 | 0/10 | 0/10 |
| substitutions 0,1 %, couverture 3 | 10/10 | 10/10 | 10/10 |
| substitutions 0,1 %, couverture 5 | 10/10 | 10/10 | 10/10 |
| substitutions 0,1 %, couverture 10 | 10/10 | 10/10 | 10/10 |
| substitutions 0,5 %, couverture 5 | 10/10 | 10/10 | 10/10 |
| substitutions 0,5 %, couverture 10 | 10/10 | 10/10 | 10/10 |
| substitutions 1 %, couverture 10 | 1/10 | 2/10 | 10/10 |
| substitutions 1 %, couverture 20 | 10/10 | 10/10 | 10/10 |
| substitutions 1 %, couverture 30 | 10/10 | 10/10 | 10/10 |
| substitutions 2 %, couverture 50 | 0/10 | 10/10 | 10/10 |
| insertions et suppressions 0,5 %, couverture 5 | 0/10 | 8/10 | 0/10 |
| insertions et suppressions 0,5 %, couverture 10 | 5/10 | 8/10 | 0/10 |
| perte de 5 % des oligos | 10/10 | 10/10 | 10/10 |
| perte de 8 % des oligos | 10/10 | 10/10 | 1/10 |
| perte de 10 % des oligos | 9/10 | 10/10 | 0/10 |
| perte de 15 % des oligos | 0/10 | 10/10 | 0/10 |
| perte de 25 % des oligos | 0/10 | 10/10 | 0/10 |
| synthèse : substitutions 0,5 %, couverture 5 | 0/10 | 0/10 | 3/10 |
| tout combiné¹, couverture 10 | 0/10 | 10/10 | 10/10 |

¹ substitutions 0,2 %, insertions 0,1 %, suppressions 0,1 %, synthèse 0,1 %, perte de 5 %.

### Lire ce tableau

- **Substitutions.** `strands` corrige deux octets par brin : il traverse 2 % de substitution.
  `fountain` n'en corrige qu'un ; il compte alors sur les lectures reçues sans erreur, et il en
  faut plusieurs par oligo dès que le taux dépasse 0,1 % : à 1 %, vingt lectures par oligo,
  quelle que soit la marge. À 2 %, seule la marge de 1,5 s'en tire, avec cinquante lectures.
- **Perte.** `fountain` traverse ce que sa marge promet, et un peu plus : à 1,1, 10 % de perte,
  neuf fois sur dix. `strands` répare 32 brins manquants sur 255 par bloc, 12,5 % en théorie ; sur
  un fichier de 1 Mio, 157 blocs, les plus malmenés décident du sort de l'archive : 5 % en
  pratique, et à 8 % un tirage sur dix à peine.
- **Insertions et suppressions.** Une lecture dont la longueur a changé est écartée, par les deux
  schémas : le décodage travaille sur des lectures de longueur exacte. À 0,5 % de chaque, il
  reste environ 20 % de lectures utilisables. `fountain` s'en tire le mieux — huit tirages sur
  dix à marge 1,5, cinq à marge 1,1 — ; `strands` jamais. Seul `hedges` les lit : voir plus loin.
- **Synthèse.** Une erreur de synthèse est dans l'oligo : toutes ses lectures la portent. Avec
  deux octets de parité, trois erreurs dans un oligo suffisent à perdre son avis, et aucun vote ne
  peut le rattraper. `strands` en traverse 0,5 % trois fois sur dix.

### La parité par oligo, devant les erreurs de synthèse

`fountain` à marge 1,1, couverture 5 sauf mention contraire :

| Scénario | parité 4 (160 nt) | parité 8 (176 nt) |
|----------|:-----------------:|:-----------------:|
| synthèse : substitutions 0,5 % | 0/10 | 10/10 |
| synthèse : substitutions 1 % | 0/10 | 9/10 |
| synthèse 1 % et substitutions 1 %, couverture 10 | 0/10 | 4/10 |
| substitutions 2 %, couverture 10 | 0/10 | 1/10 |
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
| perte 5 %, couverture 10 | 5/10 | — | — |
| perte 8 %, couverture 5 | — | 10/10 | 10/10 |
| perte 10 %, couverture 5 | — | 4/10 | — |
| perte 12 %, couverture 5 | — | 0/10 | — |
| synthèse : substitutions 1 %, couverture 3 | — | 10/10 | 10/10 |
| synthèse : substitutions 1 %, couverture 10 | 0/10 | — | — |
| synthèse : substitutions 5 %, couverture 3 | — | 0/10 | — |
| synthèse : substitutions 5 %, couverture 10 | 0/10 | — | — |

- **`grass` est le plus résistant aux substitutions** : 5 % par nucléotide à couverture 3, grâce
  à sept symboles corrigés par lecture — et un trimère hors du jeu des 47 est un effacement, qui
  ne compte pas dans les erreurs. Avec une parité de 6, il tombe à trois symboles : 1 % passe,
  2 % plus.
- **`grass` traverse 8 % de perte** à couverture 5 : 92 brins manquants sur 713 par bloc, et
  les blocs les plus malmenés décident du sort de l'archive — à 10 %, quatre tirages sur dix.
- **`goldman` a besoin de couverture**, faute de code correcteur : 5 au moins, parce qu'une base
  n'est restituée que si l'un de ses quatre brins a été lu, et qu'à couverture 3, 5 % des brins
  manquent. À couverture 10, il traverse 2 % de perte — à 5 %, une fois sur deux — et 2 % de
  substitution.
- **Les erreurs de synthèse font perdre `goldman`** : à 1 %, aucune récupération. Une erreur dans
  l'index d'un brin le fait perdre, et toutes ses lectures portent la même erreur : aucune
  couverture ne le rattrape. C'est la faiblesse du schéma publié, qui comptait sur une synthèse
  très propre.
- **Les insertions et suppressions perdent les deux** comme les autres : une lecture de 117 nt
  qui change de longueur est écartée, et il en reste environ un tiers à 0,5 % de chaque.

### Décisions souples : `ldpc`

Mêmes conditions, sur un fichier de 64 Kio d'octets incompressibles : 5 689 brins à la parité
de 20 %, 7 096 à 50 %. Un tiret : le scénario n'a pas été mesuré à cette parité.

| Scénario | `ldpc` parité 20 | `ldpc` parité 50 |
|----------|:----------------:|:----------------:|
| couverture 2 seule | 0/10 | — |
| couverture 3 seule | 10/10 | — |
| couverture 5 seule | 10/10 | — |
| substitutions 0,5 %, couverture 2 | 0/10 | — |
| substitutions 0,5 %, couverture 3 | 10/10 | — |
| substitutions 1 %, couverture 3 | 10/10 | — |
| substitutions 2 %, couverture 5 | 10/10 | 10/10 |
| substitutions 2 %, couverture 10 | 10/10 | — |
| substitutions 5 %, couverture 10 | 10/10 | 10/10 |
| substitutions 5 %, couverture 20 | 10/10 | — |
| substitutions 6 %, couverture 20 | 10/10 | — |
| substitutions 7 %, couverture 10 | 3/10 | 3/10 |
| substitutions 7 %, couverture 20 | 4/10 | — |
| substitutions 10 %, couverture 20 | 0/10 | 0/10 |
| substitutions 10 %, couverture 40 | 0/10 | 0/10 |
| synthèse : substitutions 1 %, couverture 10 | 10/10 | — |
| synthèse : substitutions 5 %, couverture 10 | 5/10 | — |
| synthèse : substitutions 7 %, couverture 10 | 0/10 | — |
| insertions et suppressions 0,5 %, couverture 10 | 4/10 | — |
| insertions et suppressions 1 %, couverture 10 | 0/10 | — |
| perte 5 %, couverture 5 | 10/10 | — |
| perte 10 %, couverture 5 | 10/10 | — |
| perte 11 %, couverture 5 | 8/10 | — |
| perte 12 %, couverture 5 | 1/10 | — |
| perte 15 %, couverture 5 | 0/10 | — |
| perte 20 %, couverture 5 | — | 10/10 |
| perte 22 %, couverture 5 | — | 10/10 |
| perte 25 %, couverture 5 | — | 7/10 |
| perte 28 %, couverture 5 | — | 0/10 |

- **La couverture compte double** : elle multiplie les lectures, et chaque lecture pèse. À 5 %
  d'erreurs, une lecture seule est à peine lisible ; dix, additionnées, donnent au LDPC un
  canal presque sans erreur. À 6 %, vingt lectures par brin suffisent ; à 7 %, vingt ne
  suffisent que quatre fois sur dix ; à 10 %, quarante ne suffisent plus.
- **La perte s'arrête où la parité le dit** : 10 % pour 20 % de brins de parité — huit tirages
  sur dix à 11 % —, 22 % pour 50 % — sept sur dix à 25 %. Un bloc qui manque plus que la parité n'en porte est refusé sans décodage.
- **Relever la parité n'aide pas contre les substitutions** : à 7 %, 50 % de parité ne sauvent
  que trois fois sur dix à couverture 10. Le goulot est alors la lecture elle-même, pas le code
  externe.
- **Les rapports d'une lecture sont bornés** : une lecture mal décodée est affirmée, et sans
  borne elle l'emporterait sur les lectures justes du même brin.

### Erreurs en salves

Sur un vrai séquençage, les erreurs se groupent : un contexte de bases en fait surgir
plusieurs d'affilée. Le canal à salves (`--burst-rate`, `--burst-length`, `--burst-error`)
les modélise. Au taux moyen égal, **une salve fait bien moins de mal qu'une erreur dispersée** :
elle ruine une lecture, que chaque schéma traite en effacement, là où l'erreur dispersée
atteint toutes les lectures.

Même fichier de 64 Kio, 10 tirages par case. Dans une salve, une base sur deux est fausse.

| Scénario, à couverture 10 | `fountain` | `strands` | `goldman` | `grass` | `ldpc` |
|--------------------------|:----------:|:---------:|:---------:|:-------:|:-------:|
| substitutions 1 % dispersées | 9/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 1 % en salves de 4, de 10, de 30 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 2 % dispersées | 0/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 2 % en salves de 4, de 10, de 30 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 5 % dispersées | 0/10 | 0/10 | 9/10 | 10/10 | 10/10 |
| substitutions 5 % en salves de 4 | 0/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 5 % en salves de 6 | 9/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 10 % en salves de 4 | 0/10 | 0/10 | 10/10 | 10/10 | 10/10 |
| substitutions 10 % en salves de 6 | 0/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| substitutions 15 % en salves de 4 | 0/10 | 0/10 | 8/10 | 10/10 | 10/10 |
| substitutions 20 % en salves de 4 | 0/10 | 0/10 | 0/10 | 10/10 | 10/10 |

À couverture 5 :

| Scénario, à couverture 5 | `fountain` | `strands` | `goldman` | `grass` | `ldpc` |
|--------------------------|:----------:|:---------:|:---------:|:-------:|:-------:|
| substitutions 5 % en salves de 4 | 0/10 | 10/10 | 9/10 | 10/10 | 10/10 |
| substitutions 10 % en salves de 4 | 0/10 | 0/10 | 0/10 | 10/10 | 10/10 |
| substitutions 10 % en salves de 30 | 8/10 | 10/10 | 10/10 | 10/10 | 10/10 |

- **`strands` bascule** : 0/10 à 5 % dispersées, 10/10 à 5 % en salves de 4. Son code
  interne corrige deux octets par brin ; une salve de quatre bases en touche un ou deux,
  l'erreur dispersée les touche tous.
- **`grass` et `ldpc` vont aussi loin en salves** : 20 % d'erreurs en salves de quatre, à
  couverture 10. Le code convolutif corrige d'office toute salve de moins de cinq bits — la
  moitié de sa distance libre.
- **`fountain` reste le plus fragile** aux substitutions, groupées ou non : sa parité par
  défaut ne corrige qu'un octet par oligo.

### Insertions et suppressions : `hedges`

Mêmes conditions, sur un fichier de 64 Kio d'octets incompressibles : 2 370 brins à ¾, 3 760 à ½
et à ⅓, 5 782 à ¼, manifestes non compris. Les taux sont ceux des trois sortes d'erreurs à la
fois, en parts égales : « indels 3 % » veut dire 1 % de substitutions, 1 % d'insertions et 1 % de
suppressions par nucléotide. Un tiret : le scénario n'a pas été mesuré à ce rendement.

| Scénario | `hedges` ¾ | `hedges` ½ | `hedges` ⅓ | `hedges` ¼ |
|----------|:----------:|:----------:|:----------:|:----------:|
| indels 1 %, couverture 10 | 10/10 | — | — | — |
| indels 1,5 %, couverture 5 | 10/10 | 10/10 | — | — |
| indels 1,5 %, couverture 10 | 10/10 | — | — | — |
| indels 1,5 %, couverture 20 | 10/10 | — | — | — |
| indels 2 %, couverture 20 | 10/10 | — | — | — |
| indels 3 %, couverture 5 | — | 10/10 | 10/10 | — |
| indels 3 %, couverture 10 | — | 10/10 | — | — |
| indels 3 %, couverture 20 | 10/10 | 10/10 | — | — |
| indels 4,5 %, couverture 10 | — | 10/10 | — | — |
| indels 6 %, couverture 5 | — | — | — | 10/10 |
| indels 6 %, couverture 10 | — | 10/10 | 10/10 | — |
| indels 6 %, couverture 20 | — | 10/10 | 10/10 | — |
| indels 9 %, couverture 20 | — | 10/10 | — | — |
| indels 9,9 %, couverture 10 | — | — | — | 10/10 |
| indels 9,9 %, couverture 20 | — | — | 10/10 | 10/10 |
| indels 12 %, couverture 10 | — | — | — | 10/10 |
| indels 12 %, couverture 20 | — | — | 10/10 | — |
| indels 15 %, couverture 10 | — | — | — | 10/10 |
| indels 15 %, couverture 20 | — | — | — | 10/10 |
| indels 18 %, couverture 20 | — | — | — | 10/10 |
| substitutions 1 %, couverture 10 | 10/10 | — | — | — |
| substitutions 3 %, couverture 10 | — | 10/10 | 10/10 | — |
| substitutions 5 %, couverture 10 | — | — | — | 10/10 |
| substitutions 6 %, couverture 20 | — | 10/10 | — | — |
| substitutions 5 % en salves de 4, couverture 10 | — | 10/10 | — | — |
| substitutions 10 % en salves de 4, couverture 20 | — | 10/10 | — | — |
| synthèse : indels 3 %, couverture 10 | — | 10/10 | — | — |
| perte 7 %, couverture 10 | — | 10/10 | — | 10/10 |
| perte 8 %, couverture 10 | 9/10 | 9/10 | 9/10 | 9/10 |
| perte 10 %, couverture 10 | — | 4/10 | — | — |

Et les autres schémas, mêmes fichier et conditions : devant 1,5 % d'erreurs des trois sortes à
couverture 10, `fountain` 0/10, `strands` 0/10, `goldman` 0/10, `ldpc` 0/10, `grass` 10/10 ;
devant 3 % d'erreurs de synthèse des trois sortes, 0/10 pour les cinq.

- **Les insertions ne coûtent pas plus que les substitutions** : à rendement égal, le schéma
  traverse la même part d'erreurs, qu'elles changent la longueur de la lecture ou non.
- **Le rendement dit ce que le schéma traverse** : à couverture 10, 1,5 % des trois sortes
  réunies au ¾, 4,5 % au ½, 6 % au ⅓ et 15 % au ¼ — et davantage avec de la couverture : à 20,
  3 %, 9 %, 12 % et 18 %.
- **Les erreurs de synthèse se corrigent comme les autres** : elles frappent l'ADN lui-même, et
  la recherche les lit à sa place. À ½, 3 % d'erreurs de synthèse des trois sortes passent,
  là où les cinq autres schémas perdent leur archive à chaque tirage.
- **Les salves ne l'inquiètent pas plus que le reste** : à ½, 10 % de substitutions en salves de
  quatre passent à couverture 20, comme 6 % dispersées.
- **La perte est celle du code externe**, et elle paie l'étroitesse de ses blocs : 255 brins,
  dont 32 de parité. À 8 %, un tirage sur dix perd un bloc, aux quatre rendements ; à 10 %,
  six.
- **Aucun fichier faux** sur les 510 tirages de cette mesure, ni dans les autres scénarios.

### L'entrelacement du schéma `ldpc`

L'entrelacement se mesure en le retirant : même archive de 64 Kio, même canal, dix tirages.
Sans lui, un brin sans lecture est une salve de 104 effacements d'affilée dans le mot de
code ; quand elle tombe sur la chaîne de parité, la propagation ne la résout que bit à bit,
le long de la chaîne, et n'en vient pas à bout en soixante allers-retours.

| Scénario | brins sans lecture | avec | sans |
|----------|:------------------:|:----:|:----:|
| couverture 5, substitutions 0,5 % | 0,6 % | 10/10 | 9/10 |
| couverture 3, substitutions 0,5 % | 5,0 % | 10/10 | 1/10 |
| couverture 3 seule | 5,0 % | 10/10 | 1/10 |
| couverture 2,5 seule | 8,3 % | 10/10 | 0/10 |
| perte 8 %, couverture 5 | 8,5 % | 10/10 | 0/10 |
| couverture 2 seule | 13,7 % | 0/10 | 0/10 |

À 5 % de brins sans lecture, l'entrelacement fait passer le taux de restitution de 1 à 10 sur
10 ; à 8 %, de 0 à 10. Les tirages réussis sans lui sont aussi plus lents : la propagation
règle la salve bit à bit. À 13,7 %, la parité ne suffit plus, entrelacée ou non.

## Refaire ces mesures

```sh
adn encode --input fichier.bin --output archive.fasta
adn simulate --archive --input archive.fasta \
    --substitution-rate 0.01 --coverage 20 --trials 10
```

Avec `--library`, `simulate` part d'un pool de bibliothèque et passe par le séquenceur :
`adn library` assemble le pool, puis `adn simulate --library --input pool.fasta`. Les mesures
de la chaîne complète sont dans [Bibliothèques Illumina](LIBRARY.md#mesures).

Sans `--archive`, `simulate` encode le fichier avec les options données, puis le dégrade —
`--algorithm hedges --code-rate 1/4` pour le schéma HEDGES.
`--synthesis-substitution-rate`, `--insertion-rate`, `--deletion-rate` et `--dropout` règlent le
canal ; `--burst-rate`, `--burst-length` et `--burst-error` y ajoutent des salves ; `--seed`
rend un tirage reproductible.

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
- Viterbi A. — *Error bounds for convolutional codes and an asymptotically optimum decoding
  algorithm*, IEEE Transactions on Information Theory 13(2):260–269, 1967.
- Bahl L. R., Cocke J., Jelinek F., Raviv J. — *Optimal decoding of linear codes for minimizing
  symbol error rate*, IEEE Transactions on Information Theory 20(2):284–287, 1974.
- Gallager R. G. — *Low-density parity-check codes*, MIT Press, 1963.
- MacKay D. J. C., Neal R. M. — *Near Shannon limit performance of low density parity check
  codes*, Electronics Letters 33(6):457–458, 1997.
- Press W. H., Hawkins J. A., Jones S. K., Schaub J. M., Finkelstein I. J. — *HEDGES
  error-correcting code for DNA storage corrects indels and allows sequence constraints*,
  PNAS 117(31):18489–18496, 2020.
- Zigangirov K. S. — *Some sequential decoding procedures using a criterion*, Problems of
  Information Transmission 2(4):13–25, 1966 ; Jelinek F. — *A fast sequential decoding
  algorithm using a stack*, IBM Journal of Research and Development 13(6):675–685, 1969.
