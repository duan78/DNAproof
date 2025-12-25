# GC-Aware Encoding for DNA Storage

## Overview

The GC-Aware encoding is an innovative approach to DNA data storage that guarantees **perfect data integrity** while making a best-effort attempt to meet biological constraints (GC content, homopolymers).

## Problem Statement

Traditional DNA encoding approaches face a fundamental dilemma:

1. **Encode first, then fix constraints**: Modifies bases after encoding to meet GC constraints, which corrupts the 2-bit mapping and breaks roundtrip capability.

2. **Pre-constrain data**: Requires preprocessing data to fit constraints before encoding, which is complex and may not always be possible.

## Solution: Header + Data + Padding Structure

Our GC-Aware encoding solves this by separating concerns:

```
┌─────────────────┬────────────────────┬──────────────────────┐
│  HEADER (25nt)  │   DATA (≤100nt)    │  PADDING (to 152nt)  │
├─────────────────┼────────────────────┼──────────────────────┤
│ Seed (8 bases)  │ Original bytes     │ GC-balanced pattern  │
│ Degree (4 bases)│ Preserved intact   │ (ignored on decode)  │
│ Address (13)    │ 2-bit mapping      │ GCTAGCTA...          │
└─────────────────┴────────────────────┴──────────────────────┘
```

### Components

#### 1. HEADER (25 bases)
- **Seed (8 bases)**: 16-bit random seed for reproducibility
- **Degree (4 bases)**: 8-bit degree value for Fountain codes
- **Addressing (13 bases)**: Reserved for future use or metadata

#### 2. DATA Section (up to 100 bases = 25 bytes max)
- **Preserved intact**: Original data encoded via standard 2-bit mapping:
  - `00` → A
  - `01` → C
  - `10` → G
  - `11` → T
- **No modifications**: Data integrity is guaranteed

#### 3. PADDING Section (to reach 152nt total)
- **Deterministic pattern**: GCTAGCTA... repeating
- **50% GC content**: Balances overall GC ratio
- **No homopolymers**: Pattern avoids runs >2
- **Ignored during decode**: Decoder skips padding using metadata

## Encoding Process

```rust
// 1. Create header with seed and degree
let header = encode_header(seed, degree);

// 2. Encode data (preserved intact)
let data = encode_data(payload);

// 3. Generate padding to reach 152nt
let padding_needed = 152 - (header.len() + data.len());
let padding = generate_gc_padding(header, data, padding_needed);

// 4. Concatenate and create sequence
let sequence = header + data + padding;
```

## Decoding Process

```rust
// 1. Extract payload length from metadata
let payload_len = sequence.metadata.chunk_size;

// 2. Calculate data section size
let data_bases = payload_len * 4; // Each byte = 4 bases

// 3. Extract only data section (skip header and padding)
let data_section = &sequence.bases[25..25 + data_bases];

// 4. Decode 2-bit mapping back to bytes
let payload = decode_bases_to_bytes(data_section);
```

## Advantages

1. **Perfect Roundtrip**: Data integrity is mathematically guaranteed
   - Original bytes preserved in DATA section
   - Padding ignored during decode
   - No data corruption possible

2. **Deterministic**: Same payload always produces same sequence
   - No randomness in encoding
   - Reproducible results

3. **Simple**: Easy to implement and understand
   - No complex iterative algorithms
   - Clear separation of concerns

4. **Best-Effort GC Balancing**: Padding improves GC content
   - Works well for typical payloads
   - May not reach ideal 40-60% for extremely biased data

## Limitations

### GC Content for Extreme Payloads

For payloads with extremely biased GC content (e.g., all 0xFF = all T's), even balanced padding may not achieve 40-60% GC because:

**Example**: All-0xFF payload (25 bytes = 100 T bases)
- Header: ~50% GC (12-13 GC bases)
- Data: 0% GC (0 GC bases - all T)
- Current: 12-13 / 125 = 10.4% GC
- Need: 76 / 152 = 50% GC
- Missing: 76 - 13 = 63 GC bases
- Available: Only 27 padding positions
- **Impossible**: 63 GC bases needed in 27 positions = 233% GC

This is a **mathematical limitation**, not an implementation flaw.

### Solution Approaches

For production use with extreme payloads:

1. **Preprocess data**: Add GC-balancing before encoding
2. **Increase sequence length**: More padding space = better GC control
3. **Accept limitation**: Use lenient constraints for extreme payloads
4. **Hybrid approach**: Use rotation (Goldman 2013) in DATA section

## Usage Example

```rust
use adn_core::codec::gc_aware_encoding::{GcAwareEncoder, GcAwareDecoder};

// Create encoder/decoder with constraints
let constraints = DnaConstraints {
    gc_min: 0.40,
    gc_max: 0.60,
    max_homopolymer: 3,
    max_sequence_length: 152,
    allowed_bases: vec![IupacBase::A, IupacBase::C, IupacBase::G, IupacBase::T],
};

let encoder = GcAwareEncoder::new(constraints.clone());
let decoder = GcAwareDecoder::new(constraints);

// Encode data
let payload = vec![0x12, 0x34, 0x56, 0x78];
let sequence = encoder.encode(payload.clone(), seed=12345, degree=5)?;

// Decode data
let recovered = decoder.decode(&sequence)?;
assert_eq!(payload, recovered); // ✅ Perfect roundtrip!
```

## Performance

- **Encoding**: O(n) where n = payload length
- **Decoding**: O(1) - constant time (just slice extraction)
- **Memory**: Minimal overhead (only padding storage)

## Scientific Background

This approach is inspired by:

1. **Erlich & Zielinski 2017** (DNA Fountain): LT codes + biochemical constraints
2. **Goldman et al. 2013**: 3-base rotation for homopolymer avoidance
3. **Grass et al. 2015**: Reed-Solomon + addressing schemes

Our innovation is the **explicit separation of data and padding**, which guarantees integrity while making best-effort constraint satisfaction.

## Future Work

1. **Adaptive padding**: Dynamically adjust padding pattern based on data GC
2. **Multi-chunk encoding**: Support larger payloads across multiple sequences
3. **Constraint-aware preprocessing**: Balance GC before encoding
4. **Optimized padding**: Use more sophisticated patterns for better GC control

## References

- [Erlich & Zielinski 2017, Science](https://science.sciencemag.org/content/357/6358/1373)
- [Goldman et al. 2013, Nature](https://www.nature.com/articles/nature11875)
- [Grass et al. 2015, Nature Biotechnology](https://www.nature.com/articles/nbt.3221)
