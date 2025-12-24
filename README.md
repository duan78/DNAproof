# ADN - Encodeur de Fichiers en ADN Virtuel

Application PoC complète pour encoder/decoder des fichiers informatiques en ADN virtuel.

## Fonctionnalités

- **Encodage**: Conversion de fichiers (txt, html, binaire) en séquences ADN virtuelles
- **Décodage**: Reconstitution des fichiers originaux depuis les séquences ADN
- **Simulation**: Modélisation des erreurs de stockage ADN
- **Visualisation**: Statistiques et analyses des séquences

## Architecture

```
adn/
├── crates/
│   ├── core/         # Logique d'encodage/décodage
│   ├── storage/      # Gestion du stockage virtuel
│   ├── simulation/   # Simulation d'erreurs
│   ├── cli/          # Interface CLI
│   └── utils/        # Utilitaires partagés
```

## Installation

```bash
cargo build --release
```

## Utilisation

```bash
# Encodage
cargo run -- encode -i input.txt -o output/

# Décodage
cargo run -- decode -i sequences.fasta -o recovered.txt

# Simulation
cargo run -- simulate -i sequences.fasta --substitution-rate 0.01 -n 100

# Visualisation
cargo run -- visualize -i sequences.fasta --format table
```

## Spécifications Techniques

- **Algorithme**: DNA Fountain (LT codes, Robust Soliton Distribution)
- **Contraintes ADN**: GC 40-60%, homopolymer < 4
- **Correction**: Reed-Solomon (255, 223)
- **Compression**: LZ4/Zstd

## License

MIT OR Apache-2.0
