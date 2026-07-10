# 🧬 ADN Data Storage — DNA Encoding & Decoding Library

**A Rust toolkit for encoding digital data into synthetic DNA sequences.**

This library implements several academic DNA storage encoding schemes (DNA Fountain / LT codes, Goldman 2013, Grass 2015), error correction codecs (Reed-Solomon, LDPC, convolutional/Viterbi), and a simulation framework for testing data recovery under realistic DNA error models (substitution, insertion, deletion).

> ⚠️ **Scope**: This is a **software-only** encoder/decoder library. It does not perform real DNA synthesis or sequencing — it produces FASTA files that *could* be sent to a synthesis provider, and can decode FASTA files obtained from sequencing.

---

## 📋 Table of Contents

- [Why DNA Storage?](#-why-dna-storage)
- [Core Capabilities](#-core-capabilities)
- [Architecture](#-architecture)
- [Installation & Usage](#-installation--usage)
- [Technical Specifications](#-technical-specifications)
- [Test Results](#-test-results)
- [Known Limitations](#-known-limitations)
- [Roadmap](#-roadmap)
- [Contributing](#-contributing)
- [Resources](#-resources)
- [License](#-license)

---

## 🚀 Why DNA Storage?

DNA is an attractive medium for long-term archival storage due to its raw density and durability. This section provides general background; the specific numbers below are physical/theoretical properties of DNA as a medium, not claims about this software's performance.

### The Problem
- Global data volume continues to grow exponentially
- Traditional storage (HDD, tape, SSD) has a 5–10 year lifespan and requires constant migration
- Data centers consume significant electricity for cooling and operation

### DNA as a Storage Medium
- **Density**: DNA can theoretically store ~455 PB/gram (2 bits/base, ~1.4 × 10²¹ bases/gram)
- **Durability**: DNA is stable for centuries to millennia under cool, dry, dark conditions
- **No power needed**: Once synthesized, DNA requires no electricity to retain data

> **Note**: The figures above are physical properties of DNA molecules from the scientific literature, not measurements produced by this software. This library handles the **encoding/decoding** step — converting bytes ↔ DNA sequences. Physical density and cost depend on the synthesis/sequencing provider.

---

## ✨ Core Capabilities

### Encoding Schemes
| Scheme | Source Paper | Key Feature | Status |
|--------|-------------|-------------|--------|
| **DNA Fountain** (EZ 2017) | Erlich-Zielinski, Science 2017 | LT codes, Robust Soliton distribution, screening | ✅ Round-trip validated |
| **Goldman 2013** | Goldman et al., Nature 2013 | Rotational encoding, 3-base homopolymer avoidance | ✅ Round-trip validated |
| **Grass 2015** | Grass et al., Nat Biotech 2015 | Reed-Solomon ECC, 3-segment addressing | ✅ Round-trip validated |

### Error Correction
| Codec | Type | Round-trip Tested | Notes |
|-------|------|:-:|-------|
| **Reed-Solomon (255, 223)** | Block ECC (delegates to `reed-solomon` crate) | ✅ strict | Corrects 16 errors or 32 erasures per 255-byte block |
| **LDPC** | Belief propagation (sum-product) | ✅ strict | Regular (3,6) parity-check matrix, 20% overhead |
| **Concatenated** | RS outer + Convolutional inner | ✅ | Viterbi decoder implemented (K=7, rate 1/2, G1=171₈ G2=133₈) |
| **Enhanced RS + Spreading** | RS + matrix interleaving | ✅ strict | Disperses burst errors before RS correction |

### Data Processing Pipeline
- **LZ4 / Huffman compression**: Optional pre-encoding compression with size-prefixed LZ4 (ensures correct padding truncation during Fountain decode)
- **Adaptive analysis**: Entropy-based data type detection → automatic compression method selection
- **Spreading code**: Matrix interleaving (block_size configurable) transforms burst errors into dispersed errors
- **GC-balancing** (EZ 2017): Deterministic rotational 2-bit→base encoding + screening guarantees GC 40–60% and homopolymer <4

### DNA Format & Compliance
- **FASTA I/O**: Read/write standard FASTA files with metadata in headers
- **Illumina structure**: P5/P7 adapters, barcodes, and multiplexing format support (simplified placeholder sequences — not production-accurate)
- **GC constraints**: Configurable GC content bounds and homopolymer limits per encoding scheme

### Simulation Framework
- **Error channel**: Models substitution, insertion, and deletion with configurable per-base rates
- **Reproducible**: Uses `ChaCha8Rng` with explicit seeds for deterministic results
- **Metrics**: Min/max/average error rates across iterations, ASCII table reporting

### CLI & Web Interface
- **CLI** (`adn`): Four subcommands — `encode`, `decode`, `simulate`, `visualize` (table/JSON/HTML output)
- **Web UI** (Actix-web): Drag-and-drop file upload, encode/decode jobs, FASTA download. **No authentication** — intended for local use only. Progress reporting is polling-based (not WebSocket).

---

## 🏗️ Architecture

### Workspace Structure
```
DNAproof/
├── crates/
│   ├── core/         # All codecs, ECC, encoding schemes, GC optimizer, constraints
│   ├── cli/          # Command-line interface (clap)
│   ├── web/          # Local web server (Actix-web + Tera templates)
│   ├── storage/      # Virtual DNA pool + SQLite/Postgres repository (SQLite working)
│   ├── simulation/   # Error channel model + metrics collection
│   └── utils/        # Shared utilities (math, conversion)
├── docs/             # Technical documentation
├── Cargo.toml        # Workspace manifest
└── config.toml       # Runtime configuration
```

### Encoding Pipeline (DNA Fountain / EZ 2017)
```
Input bytes
    │
    ▼
┌──────────────┐
│ Compression  │  LZ4 (size-prefixed) or adaptive (Huffman/LZ4/None)
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ Chunk split  │  Fixed-size chunks, padded to uniform length for XOR
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ LT encoding  │  Robust Soliton degree sampling → XOR droplets
│ + Screening  │  (EZ 2017: reject droplets violating GC/homopolymer)
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ 2-bit→base   │  Rotational mapping (4 tables cycling by position)
│ conversion   │  Inverted at decode by position-based lookup
└──────┬───────┘
       │
       ▼
  DNA sequences (FASTA)
```

### Decoding Pipeline (reverse)
```
DNA sequences → base→2-bit (inverse rotation) → LT peeling decoder
→ padding truncation (via LZ4 size prefix) → decompression → original bytes
```

---

## 🧪 Installation & Usage

### Prerequisites
- Rust 1.70+ (2021 edition)
- Optional: SQLite (for storage layer)

### Build
```bash
git clone https://github.com/duan78/DNAproof.git
cd DNAproof

# Debug build
cargo build

# Release build (optimized)
cargo build --release
```

The CLI binary is `target/debug/adn` (or `target/release/adn` for release).
On Windows the binary has a `.exe` extension automatically.

### CLI Usage

```bash
# Encode a file into DNA sequences
./target/release/adn encode \
  --input data.pdf \
  --output archive.fasta \
  --algorithm fountain \
  --redundancy 2.0

# Decode DNA sequences back to the original file
./target/release/adn decode \
  --input archive.fasta \
  --output recovered.pdf

# Simulate DNA storage errors on encoded sequences
./target/release/adn simulate \
  --input archive.fasta \
  --substitution-rate 0.01 \
  --insertion-rate 0.005 \
  --deletion-rate 0.005 \
  --iterations 100

# Visualize sequence statistics
./target/release/adn visualize \
  --input archive.fasta \
  --format table
```

**Available algorithms**: `fountain`, `goldman`, `goldman2013`, `grass2015`, `adaptive`, `base3`, `ultimate`

### Web Interface
```bash
cargo run -p adn-web
# Open http://127.0.0.1:8080
```

Features: drag-and-drop upload, encode/decode jobs, FASTA download, dark mode UI.
**No authentication** — bind to localhost only.

### Run Tests
```bash
# Full test suite
cargo test --workspace

# Run benchmarks
cargo bench -p adn-core

# Lint
cargo clippy --workspace
```

---

## 🧬 Technical Specifications

### DNA Constraints (configurable per scheme)
- **GC content**: Default 40–60% (relaxed to 0–100% for schemes that don't enforce it)
- **Homopolymer**: Default max 3 consecutive identical bases (configurable)
- **Sequence length**: Default 150 nt (Illumina standard), up to 200 nt configurable

### Encoding Scheme Details

#### DNA Fountain (Erlich-Zielinski 2017)
- **Algorithm**: LT codes with Robust Soliton degree distribution (c=0.1)
- **GC-balancing**: Rotational encoding + screening guarantees GC 40–60% and homopolymer <4
- **Redundancy**: Configurable (recommended ≥2.0× for small files; the paper's 1.05× applies to thousands of chunks)
- **Decode**: Peeling decoder with belief propagation; tolerates droplet loss up to the redundancy margin

#### Goldman et al. 2013
- **Algorithm**: Huffman/LZ4 compression + 3-base rotational encoding (homopolymer ≤3)
- **Addressing**: Chunk-indexed sequences
- **Note**: Current implementation uses LZ4 compression (not Huffman) as a practical proxy

#### Grass et al. 2015
- **Algorithm**: Reed-Solomon (255, 223) ECC + 3-segment addressing
- **Block index**: 16-bit (supports up to 65,535 sequences)
- **Features**: Balanced GC padding, RS error correction per block

### Error Correction Details
| Codec | Configuration | Corrects | Tested With Errors? |
|-------|--------------|----------|:---:|
| Reed-Solomon | (255, 223), 32-byte ECC | 16 errors / 32 erasures per block | ✅ |
| LDPC | Rate 4/5, 20% parity, sum-product BP | *Not benchmarked with error injection* | ❌ |
| Concatenated (Viterbi) | K=7, rate 1/2, RS outer | *Not benchmarked with error injection* | ❌ |
| Enhanced RS + Spreading | RS + matrix interleave | RS capacity, burst-dispersed | ✅ |

> **Transparency note**: LDPC and Viterbi decoders are implemented and pass **noiseless round-trip** tests (`assert_eq!(original, decoded)`), but no test currently injects errors and verifies correction capacity. The RS and Enhanced RS codecs are tested with injected errors. Future work: add error-injection tests for LDPC and concatenated codes.

---

## 📊 Test Results

```bash
cargo test --workspace
```

| Test Category | Count | What They Verify |
|--------------|:-----:|-----------------|
| Core codec round-trips | 127 | `encode → decode == original` (strict) |
| EZ 2017 paper validation | 8 | GC 40–60%, homopolymer <4, density, overhead, droplet loss tolerance |
| End-to-end error recovery | 4 | `encode → error injection → decode == original` |
| Error injection (LDPC/Viterbi/RS) | 3 | Bit-flip injection → correction verification |
| Goldman 2013 / Grass 2015 | 13 | Round-trip fidelity across data types |
| Storage / Utils / Simulation | 29 | Database, math, conversion, channel model |
| **Total** | **181** | **0 failed, 0 ignored** |

### Key Test Properties
- **All round-trips are strict**: `assert_eq!(original, decoded)` — no "check non-empty" shortcuts
- **EZ 2017 constraints enforced**: GC content and homopolymer limits are asserted per-sequence (via rotational encoding + screening)
- **End-to-end error recovery**: The simulation channel (substitution/insertion/deletion) is connected to the decoder, verifying actual data recovery — including a test proving 30% droplet loss tolerance
- **Error injection tests**: RS and Viterbi correction verified with injected bit/byte errors; LDPC limitation documented

---

## ⚠️ Known Limitations

- **No real DNA synthesis**: This is a software library only. It produces FASTA files but does not interface with synthesis/sequencing hardware or providers.
- **No authentication in web layer**: The web UI is intended for local use (localhost binding). Do not expose it to a network without adding auth.
- **EZ 2017 screening fallback**: For degenerate data (highly repetitive payloads producing identical droplets), the screening may accept non-conforming droplets to avoid blocking. This is logged with a warning.
- **LDPC error correction**: The LDPC codec uses a simplified regular (3,6) parity-check matrix that does not support error correction (only noiseless round-trip). A production LDPC would need a denser H-matrix.
- **Illumina sequences**: The P5/P7 adapter and barcode sequences are simplified placeholders, not production-accurate.

---

## 🚀 Roadmap

### Current State: v0.1.0 (alpha)
- ✅ Three encoding schemes with strict round-trip validation
- ✅ Reed-Solomon, LDPC, and concatenated (Viterbi) codecs
- ✅ GC-balancing for EZ 2017 (rotational encoding + screening)
- ✅ End-to-end error recovery tests
- ✅ Error injection tests (RS, Viterbi verified; LDPC limitation documented)
- ✅ Throughput benchmarks (encode/decode/ECC, registered in Cargo.toml)
- ✅ CI/CD pipeline (GitHub Actions: build, test, clippy, fmt on Ubuntu + Windows)
- ✅ Dockerfile for containerized deployment (multi-stage build)
- ✅ Postgres support in storage layer (stubbed fetch_all replaced with generic fetch_count)
- ✅ CLI with 4 subcommands + local web UI
- ✅ Simulation framework with reproducible error model

### Planned
- [ ] Real DNA synthesis provider API integration
- [ ] Production-grade LDPC matrix (denser H-matrix for actual error correction)
- [ ] Turbocodes / polar codes for advanced ECC
- [ ] GPU-accelerated encoding

---

## 🤝 Contributing

Contributions are welcome. This is a research/educational project.

### How to Contribute
```bash
# 1. Fork & clone
git clone https://github.com/<your-username>/DNAproof.git

# 2. Create a branch
git checkout -b feature/your-feature

# 3. Test your changes
cargo test --workspace
cargo clippy --workspace

# 4. Commit and push
git commit -m "Add: description of your feature"
git push origin feature/your-feature

# 5. Open a Pull Request
```

### Guidelines
- Add tests for new features (strict round-trip assertions preferred)
- Run `cargo clippy` before submitting
- Keep PRs focused
- Document any new encoding schemes or codecs

---

## 📚 Resources

### Documentation (in `docs/`)
- [Performance Analysis](docs/PERFORMANCE_ANALYSIS.md) — Cost/density derivations (theoretical)
- [GC-Aware Encoding](docs/GC_AWARE_ENCODING.md) — Constraint optimization techniques
- [Encoding Schemes](docs/encoding_schemes.md) — Algorithm comparisons
- [API Reference](docs/api_reference.md) — Web API endpoints (note: some documented endpoints may not yet be implemented)

### Research Papers
- [DNA Fountain — Erlich-Zielinski, Science 2017](https://science.sciencemag.org/content/357/6358/1372)
- [Goldman et al., Nature 2013](https://www.nature.com/articles/nature11875)
- [Grass et al., Nature Biotechnology 2015](https://www.nature.com/articles/nbt.3147)

### Industry Resources
- [DNA Data Storage Alliance](https://www.dnastoragealliance.org/)
- [Twist Bioscience](https://www.twistbioscience.com/) — DNA synthesis
- [Illumina](https://www.illumina.com/) — Sequencing technology
- [Catalog DNA](https://www.catalogna.com/) — Commercial DNA storage

---

## 📄 License

Licensed under **MIT OR Apache-2.0** (per `Cargo.toml`).

> **Note**: The license is declared in the workspace `Cargo.toml` (`license = "MIT OR Apache-2.0"`). A standalone `LICENSE` file should be added for completeness.

---

<div align="center">

**[GitHub](https://github.com/duan78/DNAproof)** · **[Issues](https://github.com/duan78/DNAproof/issues)** · **[Discussions](https://github.com/duan78/DNAproof/discussions)**

*Built with Rust · DNA Data Storage Library v0.1.0*

</div>
