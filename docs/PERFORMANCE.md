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
| Encodage `fountain` (marge 1,1) | 32 ms | 70 ms | 187 ms |
| Encodage `strands` | 33 ms | 34 ms | 41 ms |
| Décodage `fountain`, archive intacte | 3,8 ms | 13,7 ms | 63 ms |
| Décodage `strands`, archive intacte | 2,8 ms | 9,0 ms | 31 ms |
| Décodage `fountain`, 0,1 % de substitution, 3 lectures par oligo | 7,0 ms | 26 ms | 113 ms |
| Décodage `strands`, 0,5 % de substitution | 4,7 ms | 12 ms | 45 ms |

Les archives abîmées se décodent toujours : le scénario d'erreurs est choisi pour rester
décodable, et la mesure s'arrête sinon.

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
  avec DNA Fountain à marge 1,1, pour 2 bits par nucléotide possibles.

## Les briques

Prises isolément, sur un mébioctet sauf mention :

| Brique | Temps |
|---|---|
| Empreinte SHA-256 | 0,45 ms |
| CRC-32 | 1,7 ms |
| Compression zstd (octets au hasard : elle ne rapporte rien) | 3,2 ms |
| Compression zstd (texte) | 2,4 ms |
| Compression LZ4 (texte) | 1,9 ms |
| Décompression zstd (texte) | 2,1 ms |
| Reed-Solomon : parité de 223 octets, 32 de parité | 3,9 µs |
| Reed-Solomon : corriger 4 octets dans un bloc de 255 | 43 µs |
| Structure d'une goutte (32 768 blocs) | 0,22 µs |
| Test des contraintes biochimiques d'un oligo de 152 nt | 0,12 µs |
| Probabilité exacte qu'un oligo au hasard respecte les contraintes | 0,4 ms |

Le contrôle d'intégrité et la compression comptent plus que le codage lui-même : c'est le
criblage — huit candidats en moyenne pour un oligo retenu — et la certification de la marge qui
font le coût de l'encodage DNA Fountain.

## Refaire ces mesures

```sh
cargo bench -p adn-core
```

Les tests d'échelle du dépôt, ignorés par défaut, encodent et décodent des fichiers plus gros :

```sh
cargo test --release -p adn-core --test scale -- --ignored --nocapture
```
