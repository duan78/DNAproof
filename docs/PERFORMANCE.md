# Performances mesurées

Tout ce qui suit a été mesuré, pas estimé. À refaire sur votre machine : les chiffres dépendent
du processeur et de la mémoire.

- **Méthode** : `cargo bench -p adn-core` (criterion, médiane de 10 échantillons) pour les petites
  tailles ; la ligne de commande `adn`, chronométrée, pour les grandes.
- **Construction** : `--release`, avec les réglages du dépôt (liaison à l'échelle du programme,
  une unité de génération).
- **Machine** : AMD Ryzen AI Max+ 395, 32 threads, Windows.
- **Données** : octets au hasard, donc incompressibles — le cas le plus chargé en oligos.

## Petites tailles

| | 64 Kio | 256 Kio | 1 Mio |
|---|---|---|---|
| Encodage `fountain` (marge 1,1) | 40 ms | 80 ms | 215 ms |
| Encodage `strands` | 39 ms | 39 ms | 45 ms |
| Encodage `goldman` | 31 ms | 47 ms | 114 ms |
| Encodage `grass` | 68 ms | 103 ms | 267 ms |
| Encodage `ldpc` | 80 ms | 113 ms | 175 ms |
| Encodage `hedges` (rendement ½) | 4,3 ms | 12 ms | 44 ms |
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
| Décodage `hedges`, archive intacte | 11 ms | 26 ms | 94 ms |
| Décodage `hedges`, 1 % de substitution, 3 lectures par brin | 56 ms | 152 ms | 0,56 s |

Les archives abîmées se décodent toujours : le scénario d'erreurs est choisi pour rester
décodable, et la mesure s'arrête sinon.

`goldman` produit quatre fois plus de brins que les autres pour un même fichier — un Mio donne
212 000 brins — et le scénario retenu y va de trois lectures par brin : 636 000 lectures à
décoder, soit environ 1 µs chacune. `grass` écrit chaque brin en symboles de GF(47) puis le
protège par un Reed-Solomon et un criblage : son encodage est le plus lent après DNA Fountain.

Le schéma `ldpc` tient dans deux coûts. À l'encodage, la matrice de contrôle se construit une
fois par taille de bloc (38 ms), puis chaque brin cherche un sel qui le rende conforme : onze
essais en moyenne, un microseconde chacun. Au décodage, chaque lecture passe sur le treillis :
0,2 ms quand elle est fautive — c'est le log-MAP, qui pèse chaque bit —, et 15 µs quand elle est
intacte, que Viterbi reconnaît d'office. Une archive intacte se décode donc dix fois plus vite
qu'une archive à 1 % d'erreurs ; c'est le seul schéma dont le décodage compte en secondes dès
le mébioctet.

Le schéma `hedges` paie ses lectures une par une, et elles se décodent en parallèle. Une lecture
intacte coûte 12,5 µs au rendement ½, et 57 µs à 1 % de chaque sorte d'erreur ; au rendement ¾,
où la recherche hésite davantage, 153 µs. Une lecture qui résiste va au bout de son budget,
65 536 hypothèses : 2,6 ms, 200 fois une lecture intacte. Ce sont les lectures d'une autre
archive — rangée dans la mauvaise par un saut d'index — et celles qui dépassent ce que leur
rendement corrige : à ¾, une lecture sur huit à 1,5 % d'erreurs.

Ce que le parallélisme doit aux fractions de travail. Les lectures d'un brin se décodent par
fractions assez grandes pour qu'un décodeur — et sa mémoire — ne soit créé qu'une fois par
fraction. Un décodeur créé à chaque fraction volée, comme il l'était, la recherche ne passait
pas huit fils, et une archive intacte de 64 Kio prenait 87 ms. Les manifestes se cherchent en
parallèle de la même façon : chacun par une passe courte de 4 096 hypothèses, à chacun des deux
rendements de sa longueur, et la recherche entière ne vient que si aucune passe n'a abouti —
sur les 128 premières lectures candidates, au plus.

## Grandes tailles

Avec la ligne de commande, sur un fichier d'octets au hasard :

| | Taille | Temps | Mémoire maximale | Archive FASTA |
|---|---|---|---|---|
| Encodage `fountain` | 8 Mio | 1,7 s | 0,4 Go | 22 Mio |
| Encodage `fountain` | 16 Mio | 3,2 s | 0,8 Go | 43 Mio |
| Encodage `fountain` | 32 Mio | 6,9 s | 1,6 Go | 85 Mio |
| Encodage `fountain` | 64 Mio | 19 s | 3,0 Go | 171 Mio |
| Encodage `strands` | 32 Mio | 0,9 s | 0,4 Go | 98 Mio |
| Décodage `fountain` | 8 Mio | 1,1 s | 0,3 Go | |
| Décodage `fountain` | 32 Mio | 4,7 s | 1,3 Go | |
| Décodage `strands` | 32 Mio | 2,2 s | 0,9 Go | |

Chaque décodage redonne le fichier d'origine, à l'octet près.

## Lire ces chiffres

- **L'encodage DNA Fountain est l'étape coûteuse.** L'encodeur tient un décodeur à jour et
  rejoue jusqu'à 256 tirages de perte avant de livrer une archive : c'est le prix de la marge
  garantie ([schémas d'encodage](encoding_schemes.md)). `strands` encode dix fois plus vite.
- **Le décodage est rapide**, et les archives abîmées ne coûtent que deux à trois fois une
  archive intacte.
- **La mémoire suit la taille du fichier** : environ cinquante fois pour l'encodage DNA Fountain
  (1,6 Go pour 32 Mio), quarante fois pour son décodage, douze fois pour l'encodage `strands` et
  trente fois pour son décodage. Comptez-la avant d'archiver un gros fichier.
- **L'archive est plus grosse que le fichier** : environ 2,6 fois pour DNA Fountain à marge 1,1,
  3 fois pour `strands` — en FASTA replié, qui occupe le double des nucléotides. Sur un fichier
  compressible, la compression rapporte bien plus que le choix du schéma : 1,32 Mio de texte
  tient en 9 982 oligos une fois compressé par zstd, contre 39 000 pour un Mio d'octets au
  hasard.
- **Un fichier incompressible approche la limite théorique du format** : 1,41 bit par nucléotide
  avec DNA Fountain à marge 1,1, pour 2 bits par nucléotide possibles. `goldman` en est loin
  (0,34 bit par nucléotide) : son recouvrement quadruple est le prix du schéma publié.

## Les briques

Prises isolément, sur un mébioctet sauf mention :

| Brique | Temps |
|---|---|
| Empreinte SHA-256 | 0,46 ms |
| CRC-32 | 1,8 ms |
| Compression zstd (octets au hasard : elle ne rapporte rien) | 3,5 ms |
| Compression zstd (texte) | 2,9 ms |
| Compression LZ4 (texte) | 2,2 ms |
| Décompression zstd (texte) | 2,4 ms |
| Reed-Solomon GF(256) : parité de 223 octets, 32 de parité | 4,8 µs |
| Reed-Solomon GF(256) : corriger 4 octets dans un bloc de 255 | 47 µs |
| Reed-Solomon GF(47) : parité d'un brin `grass` (25 symboles, 14 de parité) | 0,57 µs |
| Reed-Solomon GF(47) : corriger 3 symboles dans un brin | 19 µs |
| Reed-Solomon GF(47) : rétablir 4 effacements dans un brin | 13 µs |
| Reed-Solomon GF(47²) : parité d'une colonne d'un bloc `grass` (621 symboles, 92 de parité) | 169 µs |
| Reed-Solomon GF(47²) : résoudre 70 effacements d'une colonne | 141 µs |
| Structure d'une goutte (32 768 blocs) | 0,25 µs |
| Test des contraintes biochimiques d'un oligo de 152 nt | 0,13 µs |
| Probabilité exacte qu'un oligo au hasard respecte les contraintes | 0,42 ms |
| Code convolutif : encoder les 148 bits d'un brin `ldpc` | 0,26 µs |
| Code convolutif : relire un brin par Viterbi (308 bits de code) | 15 µs |
| Code convolutif : le relire par log-MAP, avec un rapport par bit | 193 µs |
| LDPC : construire la matrice d'un bloc (85 176 bits d'information, 17 056 de parité) | 38 ms |
| LDPC : parité d'un bloc | 0,33 ms |
| LDPC : décoder un bloc intact | 2,2 ms |
| LDPC : décoder un bloc privé de 8 % de ses brins | 12,9 ms |
| LDPC : décoder un bloc à 1 % de bits erronés | 19,3 ms |
| Code à hachis : encoder un brin `hedges` — ¾, ½, ⅓, ¼ | 3,6 µs ; 3,1 µs ; 3,9 µs ; 3,8 µs |
| Code à hachis : lire une lecture intacte — ¾, ½, ⅓, ¼ | 17 µs ; 13 µs ; 18 µs ; 16 µs |
| Code à hachis : lire une lecture à 1 % de substitutions, d'insertions et de suppressions — ¾, ½, ⅓, ¼ | 153 µs ; 57 µs ; 40 µs ; 38 µs |
| Code à hachis : lire une lecture d'une autre archive de même longueur — ¾, ½, ⅓, ¼ | 2,9 ms ; 2,6 ms ; 2,3 ms ; 2,0 ms |

Un bloc `grass` compte dix colonnes : quand il lui manque des brins, la préparation du
rétablissement (141 µs, pour toutes les colonnes) domine, et chaque colonne revient ensuite à
une trentaine de microsecondes.

Le contrôle d'intégrité et la compression comptent plus que le codage lui-même : c'est le
criblage — huit candidats en moyenne pour un oligo retenu — et la certification de la marge qui
font le coût de l'encodage DNA Fountain. Les brins `hedges` font 200 bases à ¾ et à ½, 300 à ⅓
et à ¼ : c'est la recherche d'une lecture, pas sa longueur, qui fait son prix.

### Le code à hachis pris isolé

Hors de tout schéma — ni vote, ni code externe —, dix mille brins abîmés par point, autant de
substitutions que d'insertions et de suppressions, et relus par la recherche seule
(`cargo run --release -p adn-core --example hedges_messages`) :

| Rendement | erreurs par base | lectures exactes | messages faux | budget épuisé |
|-----------|------------------|------------------|---------------|--------------|
| ¾ | 1 % | 9 611 | 0 | 389 |
| ¾ | 1,5 % | 8 720 | 0 | 1 280 |
| ¾ | 3 % | 4 485 | 0 | 5 515 |
| ½ | 3 % | 9 917 | 0 | 83 |
| ½ | 6 % | 8 517 | 0 | 1 483 |
| ⅓ | 6 % | 9 902 | 0 | 98 |
| ⅓ | 9,9 % | 8 701 | 12 | 1 287 |
| ¼ | 9,9 % | 9 660 | 19 | 321 |
| ¼ | 12 % | 8 914 | 38 | 1 048 |
| ¼ | 15 % | 6 420 | 41 | 3 538 |

Une lecture « à budget » coûte ce que coûte une lecture étrangère. Un message faux est résolu
avec un autre contenu que celui du brin : à ¼, 1,9 pour mille à 10 % d'erreurs, 3,8 à 12 % — et
aucun à ¾, à ½ ni à ⅓ à leur taux nominal. C'est ce que le vote des lectures d'un brin et le
code externe arrêtent : aucun fichier faux dans les matrices de tolérance.

## Bibliothèques Illumina

Avec la ligne de commande, sur des fichiers d'octets au hasard :

| | Taille | Temps |
|---|---|---|
| `adn library` : assembler un pool | 4 archives de 64 Kio, un par schéma, 24 964 molécules | 42 ms |
| `adn simulate --library` : un tirage, du séquenceur au décodage des quatre échantillons | 250 084 paires de lectures | 2,6 s |
| `adn demux`, échantillons pris du pool | 250 084 paires | 1,17 s |
| `adn demux`, index déclarés, sans lecture d'index i5 | 250 084 paires | 0,94 s |
| `adn simulate --library`, pool de quatre archives `hedges` (un par rendement) : un tirage, du séquenceur au décodage | 21 962 paires | 0,4 s |
| `adn simulate --library` : un tirage, un échantillon seul | 398 921 paires (1 Mio) | 4,4 s |
| `adn demux` | 398 921 paires (1 Mio, un échantillon) | 1,86 s |

Le pool `hedges` compte des brins de 300 nt sous des lectures de 300 bases : leur lecture 2
s'arrête dix-neuf bases avant le début du brin, et `adn demux` l'écarte. Avant qu'il le fasse, le
décodeur la cherchait en vain, jusqu'au bout de son budget, et ce pool prenait 5,3 s par tirage —
0,8 s encore une fois la lecture 2 écartée, avant que la recherche et son parallélisme
s'accélèrent.

`adn demux` range environ 215 000 paires de lectures par seconde, écriture des lectures
comprise. La tolérance de la chaîne complète est mesurée dans
[Bibliothèques Illumina](LIBRARY.md#mesures).

## Les codes pris isolément

Hors de tout schéma, sur un canal binaire symétrique à décisions dures, un bloc de 102 400
bits, dix tirages par case (`cargo run --release -p adn-core --example ldpc_threshold`) :

| Rendement | erreurs corrigées | effacements corrigés |
|-----------|-------------------|----------------------|
| 0,80 | 2 % | 16 % |
| 0,67 | 4 % | 25 % |
| 0,50 | 7 % | 40 % |

La capacité du canal binaire symétrique vaut 11 % d'erreurs au rendement 0,8 : ce LDPC
s'en écarte beaucoup. Sa construction est régulière — quatre contrôles par bit, sans
distribution de degrés optimisée — et son décodage, en min-sum normalisé, perd un peu sur la
somme-produit. La matrice se reconstruisant d'une graine, une construction optimisée devrait
être décrite par peu de paramètres : c'est le chantier qui rapporterait le plus ici.

## Refaire ces mesures

```sh
cargo bench -p adn-core
```

Les durées de la ligne de commande — décodage `hedges`, chaîne Illumina — viennent de
`measure/timings.py`, et les tirages de la chaîne de `measure/library_pools.py`
([`measure/`](../measure/README.md)).

Les tests d'échelle du dépôt, ignorés par défaut, encodent et décodent des fichiers plus gros :

```sh
cargo test --release -p adn-core --test scale -- --ignored --nocapture
```
