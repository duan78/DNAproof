# Journal des changements

Le format retenu est celui de [Keep a Changelog](https://keepachangelog.com/fr/), et les versions
suivent la [gestion sémantique](https://semver.org/lang/fr/).

## [0.2.0] — 2026-11-23

Cette version réécrit le format d'archive, le décodeur, la simulation, la ligne de commande et le
serveur web. Les archives produites par la 0.1 ne sont plus lues (voir *Retiré*).

### Ajouté

- **Format d'archive « ADN2 », version 2**, décrit dans [`docs/FORMAT.md`](docs/FORMAT.md). Une
  archive se décode depuis ses seules séquences : le schéma, la compression et les paramètres de
  codage sont portés par des oligos « manifestes », et l'empreinte SHA-256 du fichier voyage dans
  l'enveloppe. Un fichier n'est jamais rendu si son empreinte ne correspond pas.
- **Schéma `strands`** (brins codés) : brins de 160 nt adressés, parité Reed-Solomon interne de
  4 octets par brin, code externe de 32 brins de parité par bloc de 223. Le code externe
  contrôle et corrige chaque bloc : un brin lu à tort est rétabli, deux erreurs valant un brin
  manquant.
- **Schéma `fountain`** (DNA Fountain) réécrit : graine de 4 octets, charge utile de 32 octets,
  criblage aux contraintes biochimiques, marge de perte certifiée par l'encodeur, paramètre `c`
  choisi automatiquement pour les petites archives. Au décodage, les lectures reçues sans erreur
  votent par goutte, et la résolution se fait d'abord avec les seules gouttes certaines : une
  lecture fausse que la parité croit exacte ne peut pas corrompre l'archive.
- **Schéma `goldman`** (Goldman et al., 2013) : code de Huffman ternaire, jamais deux bases
  identiques de suite, fenêtres de 100 nt décalées de 25 — chaque base du message est lue dans
  quatre brins de 117 nt, et le vote la restitue. Brins de 117 nt, environ 2,6 Mo au plus,
  identifiant de fichier (`--file-id`) pour mélanger plusieurs archives dans un même tube. Comme
  dans l'article, la teneur en GC n'y est pas garantie par brin.
- **Schéma `grass`** (Grass et al., 2015) : Reed-Solomon sur le corps à 47 éléments, un symbole
  par groupe de trois bases choisi pour qu'aucune suite de plus de trois bases n'apparaisse.
  Brins de 117 nt, parité interne réglable (`--rs-parity`, 14 symboles par défaut : sept symboles
  erronés par lecture), code externe d'un Reed-Solomon sur GF(47²) par bloc de 713 brins : 92
  brins de parité pour 621 de données.
- **Reed-Solomon sur GF(47) et GF(47²)** : code d'évaluation générique, décodage de Gao, et
  rétablissement des effacements partagés entre plusieurs colonnes.
- **`adn verify`** : contrôle une archive (contraintes, manifeste, décodabilité) sans rien écrire.
- **`adn simulate`** : dégrade une archive (perte, substitutions, insertions, suppressions,
  couverture) et tente de la relire.
- **`adn visualize`** : statistiques d'un ensemble de séquences, en tableau, JSON ou HTML.
- **Bibliothèques Illumina** (`adn_core::library`) : molécules au format Nextera — P5, index i5
  et i7, extrémité mosaïque, sites d'amorce —, index du kit Nextera (`N501`–`N508`,
  `N701`–`N712`) ou séquences libres, bibliothèque vérifiée (deux index à trois bases l'un de
  l'autre au moins, jamais compléments inversés), démultiplexage à une erreur par index dans
  les deux sens de lecture, coupe des adaptateurs.
- **`adn library`** : assemble des archives en un pool de molécules, un échantillon par
  archive.
- **`adn demux`** : sépare les lectures d'un séquenceur par échantillon (FASTQ ou FASTA),
  coupe les adaptateurs, remet la lecture 2 dans le sens de l'insert et écarte les lectures 2
  arrêtées dans l'insert — sans le début du brin, elles ne servent à rien ; `--truth` confronte
  le rangement à l'origine des lectures simulées.
- **Séquenceur simulé** (`adn_simulation::Sequencer`) et **`adn simulate --library`** : lectures
  appariées, lectures d'index, erreurs croissantes le long de la lecture et qualités Phred,
  sauts d'index, jusqu'au décodage de chaque échantillon.
- **FASTQ** : lecture (reconnue au `@` initial, qualités contrôlées par leur longueur) et
  écriture (`adn_core::fasta::FastqWriter`), par la ligne de commande comme par le serveur
  web.
- **[`docs/LIBRARY.md`](docs/LIBRARY.md)** : le format, le séquenceur, les commandes et les
  mesures.
- **`--json`** sur toutes les commandes.
- **Empreinte attendue** : `adn decode --expect-sha256` et le champ `expected_sha256` de l'API
  web refusent un fichier d'une autre empreinte.
- **Serveur web** : liste des tâches (`GET /api/jobs`), suppression (`DELETE /api/jobs/{id}`,
  refusée tant que la tâche tourne), état du serveur (`GET /api/stats`), noms d'hôtes acceptés
  derrière un proxy inverse (`server.allowed_hosts`), nom du fichier d'origine conservé et
  proposé au téléchargement, pages qui fonctionnent sans accès à Internet.
- **Reprise après redémarrage** : quand la base de données est activée, les archives restent
  listées et téléchargeables après un redémarrage ; un fichier de résultat disparu est réécrit
  depuis les oligos rangés en base.
- **Banc de mesures** : `cargo bench -p adn-core` ; les résultats sont dans
  [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md).
- **Documentation refaite** : spécification du format, schémas et tolérance aux erreurs mesurée,
  performances, guide de production, référence de l'API web.
- **Schéma `ldpc`** : un code convolutif par brin — (171, 133) en octal, longueur de contrainte
  7, rendement ½ —, décodé sur le treillis par l'algorithme log-MAP, avec un rapport de
  vraisemblance par bit ; un LDPC par bloc de 819 brins, reconstruit d'une graine, décodé par
  propagation de croyances (min-sum normalisé) ; un entrelacement entre les deux. Brins de
  154 nt, parité réglable (`--rs-parity`, de 5 à 100 pour cent, 20 par défaut). C'est le
  schéma qui traverse le plus d'erreurs dispersées : 5 % de substitution à couverture 10, et
  5 % d'erreurs de synthèse — que nul autre ne traverse.
- **Erreurs en salves** dans la simulation : `--burst-rate`, `--burst-length` et
  `--burst-error` décrivent un canal à deux états, où les erreurs se groupent.
- **Codes pris isolément** (`adn_core::convolutional`, `adn_core::ldpc`) : Viterbi à entrées
  souples, log-MAP, LDPC de poids de colonne réglable ; `examples/ldpc_threshold.rs` mesure
  leurs seuils.
- **Schéma `hedges`** : HEDGES (Press et al., 2020), code à hachis par brin, décodé par
  une recherche best-first — le seul schéma qui corrige les **insertions et les
  suppressions** en même temps que les substitutions. Quatre rendements
  (`--code-rate`) : ¾, ½, ⅓ et ¼, pour environ 1 %, 3 %, 6 % et 10 % d'erreurs des trois
  sortes réunies par base. Un Reed-Solomon (255, 223) appliqué en diagonale entre les brins
  restitue jusqu'à 32 brins manquants par bloc, ou en corrige 16 de faux ; les lectures
  d'un même brin votent. L'oligo manifeste est codé de même, derrière l'étiquette
  `HEDG` : l'archive se lit encore quand insertions et suppressions abîment jusqu'à
  ses manifestes. Mesuré avec `adn simulate`, 64 Kio, dix tirages indépendants par
  cas : à couverture 10, le fichier est restitué dix fois sur dix avec 1,5 % d'erreurs des
  trois sortes réunies par base au rendement ¾, 4,5 % au ½, 6 % au ⅓, 15 % au ¼ ; à
  couverture 20, 3 %, 9 %, 12 % et 18 %. Les autres schémas, eux, échouent à 1,5 %
  (`grass` s'en tire à cette couverture), et tous perdent leur archive à 3 % d'erreurs de
  synthèse des trois sortes. Sept pour cent de brins perdus passent, 8 % neuf fois sur
  dix. Sur les 510 tirages de cette mesure, aucun fichier faux.
- **Code à hachis pris isolément** (`adn_core::hedges`) : l'encodeur et la recherche.
  S'ajoutent à l'article trois garde-fous : une chaîne qui suppose trop d'erreurs est
  abandonnée avant d'épuiser le budget, la queue de zéros est forcée et contrôle
  encore, et un rival presque à égalité fait échouer la lecture plutôt que de laisser
  passer un message faux. `cargo run --release -p adn-core --example hedges_messages` mesure
  ce que la recherche rend de lectures abîmées : à ¼, 1,9 lecture sur mille rend un message
  faux à 10 % d'erreurs, 3,8 à 12 % — le vote et le code externe les arrêtent.
- **Mesures reproductibles** (`measure/`) : les scripts Python qui produisent la matrice de
  `hedges` et celles de la chaîne Illumina, avec `tables.py` pour les mettre en tableau et
  `check_docs.py` pour contrôler les liens et les tableaux des documents.
- Licence Apache 2.0 : le texte accompagne désormais la licence MIT.

### Modifié

- **Le décodage de `hedges` va trois à dix fois plus vite, à résultat identique.** Les lectures
  se décodent en parallèle par fractions assez grandes pour qu'un décodeur ne soit créé qu'une
  fois par fraction : créé à chaque fraction volée, la recherche ne passait pas huit fils, et
  une archive intacte de 64 Kio prenait 87 ms — 11 ms aujourd'hui. La recherche garde ses
  hypothèses dans un tas tant qu'elles sont peu nombreuses, dans des seaux — un par score, en
  travail constant — dès qu'elles le deviennent trop : une lecture sans issue coûte 2,6 ms au
  lieu de 4,3. Les manifestes se cherchent en parallèle, avec un décodeur par rendement gardé
  d'une lecture à l'autre. L'ordre dans lequel la recherche développe ses hypothèses est figé
  par des sommes de contrôle : rien ne change, ni le message rendu, ni son prix.
- **Simulation : les tirages d'une même commande sont indépendants.** La clé des tirages
  d'un oligo était son rang combiné par OU exclusif avec la graine : d'une graine voisine à
  l'autre — d'un tirage à l'autre —, les oligos sans lecture ne faisaient que se déplacer, et
  la charge de chaque bloc d'un code externe restait la même. Les matrices de tolérance ont
  été reprises : devant la perte ou une faible couverture, une case mesurait un seul tirage.
  Les cas limites y montrent leur vraie probabilité — `strands` traverse 5 % de perte, et
  8 % un tirage sur dix ; `ldpc` traverse 6 % de substitution à couverture 20, et 7 % quatre
  fois sur dix ; `goldman` traverse 2 % de perte, et 5 % une fois sur deux ; `grass` 8 %.
  Les scénarios à taux d'erreurs, eux, étaient déjà des tirages distincts, et ne bougent
  qu'à un tirage près.
- Les archives du schéma `hedges` refusent d'emblée des contraintes plus étroites que ce
  que le code garantit (bornes de GC, deux bases identiques au plus), comme `goldman` et
  `grass` le faisaient pour les leurs.
- **Version minimale de Rust : 1.94** (précédemment 1.87, non vérifiée). Elle est imposée par
  `sqlx 0.9`, qui corrige une vulnérabilité de la bibliothèque SQLite embarquée.
- `Cargo.lock` est suivi dans le dépôt ; la construction d'une image s'y tient (`--locked`).
- Les noms de schéma acceptés sont `fountain`, `strands`, `goldman`, `grass`, `ldpc` et `hedges`
  (`goldman2013`, `grass2015`, `viterbi` et `indels` sont des alias).
- La compression automatique utilise zstd (LZ4 reste au choix).
- La limite d'envoi du serveur web passe de 100 Mio à 25 Mio.
- Les résultats du serveur web sont conservés 24 heures par défaut (une heure auparavant, et
  seulement au moment d'une nouvelle requête).
- `--rs-parity` vaut, par défaut, la parité du schéma : deux octets pour DNA Fountain, quatorze
  symboles pour grass. `adn verify` ne reproche pas au schéma goldman sa teneur en GC.

### Retiré

- Les schémas `adaptive`, `base3` et `ultimate`, et les anciens `goldman` et `grass2015` : ils
  n'implémentaient pas ce que leur nom annonçait. Les schémas `goldman` et `grass` sont réécrits à
  neuf d'après les articles (voir *Ajouté*) ; les trois autres noms sont refusés avec un message
  explicite.
- Les codecs LDPC et convolutif (Viterbi), « Reed-Solomon renforcé avec entrelacement » et les
  séquences Illumina. Le LDPC ne corrigeait aucune erreur, le code convolutif n'était jamais
  testé avec des erreurs injectées, les séquences Illumina étaient fictives. Le code convolutif,
  le LDPC, l'entrelacement et les bibliothèques Illumina sont réécrits à neuf (voir *Ajouté*).
- La lecture des archives de la version 0.1.
- `POST /api/encode/stream` : il ne transmettait rien en flux, et contournait les contrôles.
- CORS : l'API ne se lit que depuis les pages du serveur.
- La feuille de style chargée depuis un CDN : les pages sont autonomes.
- Les dépendances `actix-cors`, `chrono` (non maintenu), `indicatif` et `tabled` (inutilisées).
- Quatre fichiers de résultats qui étaient suivis dans `crates/web/uploads/`.
- `docs/GC_AWARE_ENCODING.md` et `docs/PERFORMANCE_ANALYSIS.md` : ils décrivaient un code retiré
  et des chiffres invérifiables. Remplacés par les [schémas d'encodage](docs/encoding_schemes.md)
  et les [performances mesurées](docs/PERFORMANCE.md).

### Sécurité

- `cargo audit` ne signale plus rien : la version 0.1 en comptait 18, dont une divulgation de
  fichiers par noms courts Windows (`actix-files`), du passage de requêtes (`actix-http`), et une
  lecture de mémoire non initialisée dans SQLite.
- Le serveur web n'active que les fonctionnalités d'actix qu'il utilise : ni cookies, ni HTTP/2,
  ni compression des réponses, ni websockets.
- Écoute sur la boucle locale par défaut, et refus des requêtes dont l'hôte n'est pas la machine
  elle-même : un site tiers ne peut pas, en faisant rebinder son domaine, lire les résultats.
- Les fichiers restitués partent en pièces jointes, avec `X-Content-Type-Options: nosniff`.
- Le nom d'un fichier envoyé est nettoyé avant de servir au téléchargement ; l'identifiant d'une
  tâche doit être un UUID.
- Les clés inconnues de `config.toml` arrêtent le démarrage, au lieu d'être ignorées.

## [0.1.0] — 2026-08-15

Première version publiée : ligne de commande `adn`, serveur web, stockage SQLite et PostgreSQL.
