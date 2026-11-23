# Les schémas d'encodage

Deux schémas répartissent un fichier en oligos. Ils partagent la même enveloppe, les mêmes
manifestes et les mêmes contraintes biochimiques ([format](FORMAT.md)) ; ils diffèrent par la
façon dont les oligos portent les données, donc par ce qu'ils savent traverser.

- **`fountain`** — DNA Fountain (Erlich et Zielinski, 2017) : des gouttes sans ordre ni adresse.
- **`strands`** — brins codés : des brins adressés, protégés par deux niveaux de Reed-Solomon.

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

## Choisir

| Vous avez… | Prenez |
|------------|--------|
| un petit fichier (moins de quelques centaines de kilooctets) | `strands` |
| un fichier de plus de 420 Mio | `fountain` |
| un canal qui perd des oligos | `fountain`, en relevant la marge (`--redundancy 1.5` voire plus) |
| un canal à substitutions, peu de pertes | `strands` : il traverse 2 % de substitution |
| un séquençage à insertions et suppressions | `fountain` avec une marge large et beaucoup de couverture, ou un séquençage meilleur : les lectures qui changent de longueur sont perdues |
| une synthèse bruitée | `fountain` avec `--rs-parity 8` |

## Tolérance aux erreurs

Mesurée avec `adn simulate`, sur un fichier de 1 Mio d'octets incompressibles, 10 tirages par
case. « 10/10 » : le fichier est restitué dans les dix tirages. **Sur les 670 tirages, aucun
fichier faux n'a jamais été rendu** : un échec est toujours annoncé.

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
  Science 355:950–954, 2017.
- Grass R. N., Heckel R., Puddu M., Paunescu D., Stark W. J. — *Robust Chemical Preservation of
  Digital Information on DNA in Silica with Error-Correcting Codes*, Angewandte Chemie
  International Edition 54:2552–2555, 2015.
- Luby M. — *LT codes*, FOCS 2002.
- Goldman N. et al. — *Towards practical, high-capacity, low-maintenance information storage in
  synthesised DNA*, Nature 494:77–80, 2013.
