# Schémas d'Encodage ADN

Ce document décrit les différents schémas d'encodage ADN implémentés dans ce projet, basés sur des publications scientifiques.

## Table des Matières

- [Standards Publiés](#standards-publiés)
  - [Goldman et al. 2013](#goldman-et-al-2013)
  - [Erlich-Zielinski 2017](#erlich-zielinski-2017)
  - [Grass et al. 2015](#grass-et-al-2015)
- [Implémentations Custom](#implémentations-custom)
- [Guide de Choix d'Algorithme](#guide-de-choix-dalgorithme)
- [Performances Comparées](#performances-comparées)

---

## Standards Publiés

### Goldman et al. 2013

**Référence**: *Nature 2013* "Towards practical, high-capacity, low-maintenance information storage in synthesized DNA"

**Algorithme**:
- Encodage 3-base rotation (pas 2-bit fixe)
- Addressing 8-base (16-bit) supportant jusqu'à 65,535 séquences
- Segments alternés addressing/data
- Contraintes GC équilibrées avec rotation
- Oligos 100-150nt

**Performance**:
- Densité: ~1.6 bits/base
- Overhead: ~2.5× (addressing + rotation)
- Tolérance erreur: Modérée

**Quand l'utiliser**:
- ✅ Meilleur pour: Fichiers texte, JSON, données répétitives
- ✅ Petits à moyens fichiers (<100KB)
- ⚠️ Éviter pour: Données déjà compressées

**Exemple**:
```bash
adn encode --input document.txt --output sequences/ --algorithm goldman2013
```

---

### Erlich-Zielinski 2017 (DNA Fountain)

**Référence**: *Science 2017* "DNA Fountain enables a robust and efficient storage architecture"

**Algorithme**:
- DNA Fountain codes (LT codes + distribution Robust Soliton)
- Paramètres validés: c=0.1, δ=0.5
- Contraintes biochemical strictes: GC 40-60%, homopolymer <4
- Oligos 152nt
- Compression LZ4/Zstd intégrée

**Performance**:
- Densité: ~1.92 bits/base (la plus élevée)
- Overhead: ~1.03-1.07× (minimum théorique)
- Tolérance: Excellente (peut perdre >30% oligos)

**Quand l'utiliser**:
- ✅ Meilleur pour: Fichiers volumineux, stockage archival
- ✅ Images, PDF, binaires
- ✅ Environnements avec taux d'erreur élevé
- ⚠️ Éviter pour: Petits fichiers <1KB (overhead)

**Exemple**:
```bash
adn encode --input archive.tar --output sequences/ --algorithm fountain --redundancy 1.05
```

---

### Grass et al. 2015

**Référence**: *Nature Biotechnology 2015* "Robust chemical preservation of digital information on DNA in silica with error-correcting codes"

**Algorithme**:
- Addressing 3-segments (byte_offset, bit_offset, block_index)
- Reed-Solomon (255, 223) comme code interne (version simplifiée actuelle)
- 4% redondance logique
- Oligos 124nt
- Rotation pour éviter homopolymères

**Performance**:
- Densité: ~1.5 bits/base
- Overhead: ~4% logique + redondance physique
- Tolérance: Excellente (ECC multi-couches avec RS)

**Quand l'utiliser**:
- ✅ Meilleur pour: Archivage long-terme, haute fiabilité
- ✅ Données critiques nécessitant une forte redondance
- ⚠️ Éviter pour: Applications coûts-sensibles (plus d'oligos)

**Exemple**:
```bash
adn encode --input critical_data.bin --output sequences/ --algorithm grass2015
```

---

## Implémentations Custom

### DNA Fountain (Custom)

Implémentation originale de DNA Fountain avec:
- Distribution Robust Soliton simplifiée
- Parallelisme avec Rayon
- Contraintes GC configurables

**Usage**: Pour tests et développement, utiliser EZ 2017 pour production.

### Goldman (Simple 2-bit)

Version simple de l'algorithme Goldman:
- Encodage 2-bit fixe (00→A, 01→C, 10→G, 11→T)
- Pas de rotation
- Pas de contrôle des contraintes

**Usage**: Tests et compatibilité uniquement. Utiliser Goldman 2013 pour production.

---

## Guide de Choix d'Algorithme

### Par Type de Fichier

| Type | Meilleur Algorithme | Pourquoi |
|------|---------------------|----------|
| **TXT, CSV, JSON** | Goldman 2013 | Rotation + addressing efficace pour données répétitives |
| **PDF, Images** | Erlich-Zielinski 2017 | Déjà compressés, densité maximale |
| **Binaires** | Grass 2015 | Fiabilité maximale avec ECC multi-couches |
| **Archives TAR/ZIP** | Erlich-Zielinski 2017 | Haute densité + excellente tolérance erreur |

### Par Cas d'Usage

| Cas d'Usage | Algorithme | Raison |
|-------------|-----------|--------|
| **Stockage archival** | Erlich-Zielinski 2017 | Densité max + tolérance erreur |
| **Archivage long-terme** | Grass 2015 | ECC Reed-Solomon pour fiabilité maximale |
| **Données critiques** | Grass 2015 | Redondance multi-couches |
| **Prototypage** | Goldman 2013 | Simple et efficace |
| **Démo/PoC** | Erlich-Zielinski 2017 | Meilleure densité = moins d'ADN |
| **Budget limité** | Goldman 2013 | Moins d'oligos que Grass 2015 |

### Par Taille de Fichier

| Taille | Recommandation |
|--------|----------------|
| **< 1KB** | Goldman 2013 (overhead minimum) |
| **1KB - 100KB** | Goldman 2013 ou Erlich-Zielinski 2017 |
| **100KB - 10MB** | Erlich-Zielinski 2017 |
| **> 10MB** | Erlich-Zielinski 2017 (densité optimale) |

---

## Performances Comparées

### Densité d'Information

| Algorithme | Bits/Base | Bases/octet théorique | Efficacité |
|-----------|-----------|----------------------|------------|
| **Erlich-Zielinski 2017** | 1.92 | 4.17 | ★★★★★ |
| **Goldman 2013** | 1.6 | 5.0 | ★★★★☆ |
| **Grass 2015** | 1.5 | 5.33 | ★★★☆☆ |
| **Goldman (Simple)** | 2.0 | 4.0 | ★★★★★ (sans contraintes) |

**Note**: Plus "bases/octet" est faible, meilleure est l'efficacité.

### Overhead (Redondance)

| Algorithme | Overhead | Sequences/KB (données aléatoires) |
|-----------|----------|-----------------------------------|
| **Erlich-Zielinski 2017** | 1.03-1.07× | ~250 |
| **Goldman 2013** | ~2.5× | ~500 |
| **Grass 2015** | ~4% + RS | ~260 |
| **Goldman (Simple)** | 1× | ~250 |

### Tolérance aux Erreurs

| Algorithme | Substitution | Insertion/Délétion | Perte oligos | Classement |
|-----------|-------------|-------------------|--------------|------------|
| **Grass 2015** | Excellent | Excellent | Bon (4%) | ★★★★★ |
| **Erlich-Zielinski 2017** | Bon | Modéré | Excellent (30%+) | ★★★★☆ |
| **Goldman 2013** | Modéré | Modéré | Faible | ★★★☆☆ |

### Synthétisabilité (Coût)

| Algorithme | Coût/oligo | Synthétisabilité | Recommandation |
|-----------|-----------|------------------|----------------|
| **Erlich-Zielinski 2017** | ★★★★★ | GC 40-60%, homo <4 | Meilleure pour synthèse |
| **Goldman 2013** | ★★★★☆ | Rotation équilibrée | Bonne |
| **Grass 2015** | ★★★☆☆ | Plus d'oligos = plus cher | OK pour archivage |

---

## Utilisation API

### Rust

```rust
use adn_core::{Encoder, EncoderConfig, EncoderType};

// Erlich-Zielinski 2017
let config = EncoderConfig {
    encoder_type: EncoderType::ErlichZielinski2017,
    redundancy: 1.05,
    ..Default::default()
};

let encoder = Encoder::new(config)?;
let sequences = encoder.encode(&file_data)?;

// Goldman 2013
let config = EncoderConfig {
    encoder_type: EncoderType::Goldman2013,
    ..Default::default()
};

let encoder = Encoder::new(config)?;
let sequences = encoder.encode(&file_data)?;

// Grass 2015
let config = EncoderConfig {
    encoder_type: EncoderType::Grass2015,
    constraints: DnaConstraints {
        gc_min: 0.0,
        gc_max: 1.0,
        max_homopolymer: 150,
        max_sequence_length: 200,
        allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
    },
    ..Default::default()
};

let encoder = Encoder::new(config)?;
let sequences = encoder.encode(&file_data)?;
```

### CLI

```bash
# Erlich-Zielinski 2017 (recommandé)
adn encode --input file.txt --output sequences/ \
  --algorithm fountain --redundancy 1.05

# Goldman 2013
adn encode --input file.txt --output sequences/ \
  --algorithm goldman2013

# Grass 2015
adn encode --input file.txt --output sequences/ \
  --algorithm grass2015
```

---

## Recommandations Finales

### Pour la Production

1. **Par défaut**: Utiliser **Erlich-Zielinski 2017** (Fountain)
   - Meilleure densité
   - Excellente tolérance aux erreurs
   - Contraintes biochemical optimales

2. **Pour archivage long-terme**: Utiliser **Grass 2015**
   - Reed-Solomon pour ECC
   - Redondance multi-couches
   - Fiabilité maximale

3. **Pour données texte/répétitives**: Utiliser **Goldman 2013**
   - Rotation efficace
   - Addressing simple
   - Bon compromis

### Pour le Développement

- Utiliser les tests unitaires dans `crates/core/src/codec/`
- Voir `crates/core/tests/roundtrip_tests.rs` pour exemples
- Consulter les papiers originaux pour détails théoriques

---

## Références Scientifiques

1. **Goldman et al. 2013**: "Towards practical, high-capacity, low-maintenance information storage in synthesized DNA" - *Nature* 494, 77-80
2. **Erlich & Zielinski 2017**: "DNA Fountain enables a robust and efficient storage architecture" - *Science* 355, 950-954
3. **Grass et al. 2015**: "Robust chemical preservation of digital information on DNA in silica with error-correcting codes" - *Nature Biotechnology* 33, 884-890
