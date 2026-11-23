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
- Licence Apache 2.0 : le texte accompagne désormais la licence MIT.

### Modifié

- **Version minimale de Rust : 1.94** (précédemment 1.87, non vérifiée). Elle est imposée par
  `sqlx 0.9`, qui corrige une vulnérabilité de la bibliothèque SQLite embarquée.
- `Cargo.lock` est suivi dans le dépôt ; la construction d'une image s'y tient (`--locked`).
- Les noms de schéma acceptés sont `fountain`, `strands`, `goldman` et `grass` (`goldman2013` et
  `grass2015` sont des alias).
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
  le LDPC, l'entrelacement, HEDGES et le format Illumina sont au programme du projet, pour être
  reconstruits pour de vrai (voir la feuille de route du README).
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
