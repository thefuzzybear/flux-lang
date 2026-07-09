# Advanced Decorators — HFT Market Making with Memory Layout Control

A comprehensive showcase of Flux's decorator system applied to a high-frequency
market making strategy. Each struct demonstrates a different memory layout
decorator with inline documentation explaining the use case.

## Decorators Reference

| Decorator | Rust Output | Use Case |
|-----------|-------------|----------|
| `@aligned(64)` | `#[repr(align(64))]` | Cache-line alignment, prevents false sharing |
| `@packed` | `#[repr(packed)]` | Zero padding, minimal memory for wire formats |
| `@immutable` | Compile-time mutation rejection | Frozen config, prevents accidental corruption |
| `@volatile` | `read_volatile`/`write_volatile` | Shared-memory feeds from external processes |
| `@heap` | `Box<T>`, pass by `&T` | Large structs that don't fit on the stack |
| `@prefetch` | `_mm_prefetch` intrinsics | Pre-load into L1 cache before access |
| `@streaming` | Non-temporal stores | Write-once data that shouldn't pollute cache |
| `@soa` | Struct-of-arrays transform | SIMD vectorization of per-field operations |
| `@pool(N)` | Pre-allocated slab + free-list | O(1) alloc/free for order lifecycle |
| `@simd(256)` | `#[repr(align(32))]` | AVX2 register alignment for vectorized math |
| `@bitfield` | Bit shift/mask packing | Dense flag storage in a single u64 |
| `@zero_init` | `std::mem::zeroed()` | Guaranteed clean state for accumulators |
| `@stack` | `#[derive(Clone, Copy)]` | Explicit stack allocation (default behavior) |

## Compatibility Rules

Some decorators cannot be combined:

| Incompatible Pair | Reason |
|-------------------|--------|
| `@packed` + `@aligned` | Contradictory: packed removes padding, aligned adds it |
| `@soa` + `@packed` | SoA transforms layout, packed constrains it |
| `@stack` + `@heap` | Mutually exclusive allocation strategies |
| `@pool` + `@heap` | Pool IS the allocator, can't also be heap |
| `@pool` + `@stack` | Pool manages its own memory, not the stack |
| `@bitfield` + `@soa` | Bitfield packs into u64, incompatible with SoA |
| `@immutable` + `@volatile` | Immutable = no writes, volatile = external writes |

## Project Structure

```
demos/advanced_decorators/
├── strategy.flux                # Main strategy — compiles and runs today
├── types/                       # Reference library (standalone documentation)
│   ├── market_state.flux        # @aligned, @volatile, @prefetch — with docs
│   ├── orders.flux              # @pool, @bitfield, @packed — with docs
│   ├── config.flux              # @immutable, @zero_init — with docs
│   └── performance.flux         # @simd, @soa, @streaming, @heap, @stack — with docs
└── README.md                    # This file
```

The main `strategy.flux` contains all types inline (required today since local module
import of struct types is not yet wired). The `types/` directory provides the same
structs as standalone reference files with comprehensive documentation — each file
can be individually parsed with `flux check` as a library file.

### Import Patterns (working today)

```flux
# Standard library — resolves from std/ directory
from market::l1 import {Quote, calc_spread, calc_mid}
from market::l2 import {Book, book_spread_bps, book_microprice, book_imbalance}
```

### Import Patterns (future — when local struct imports are wired)

```flux
# Project-local modules — will resolve relative to the strategy file
from types::market_state import {MarketState, SignalVector}
from types::orders import {LiveOrder, OrderFlags, TradeRecord}
from types::config import {StrategyConfig, SessionStats}
from types::performance import {PriceVector, TickFeature, FillLog}
```

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                 on bar (hot path)                    │
├─────────────────────────────────────────────────────┤
│                                                     │
│  SharedFeedState (@volatile)                        │
│      ↓ read latest market data                      │
│  MarketState (@aligned)                             │
│      ↓ compute signals                              │
│  SignalVector (@prefetch)                            │
│      ↓ decision                                     │
│  LiveOrder (@pool) ← alloc from slab                │
│      ↓ execution                                    │
│  FillLog (@streaming) → write to audit trail        │
│      ↓ accounting                                   │
│  SessionStats (@zero_init) ← accumulate P&L        │
│                                                     │
│  StrategyConfig (@immutable) — frozen parameters    │
│  OrderFlags (@bitfield) — dense per-order status    │
│  PriceVector (@simd) — vectorized price math        │
│  TradeRecord (@packed) — wire-format storage        │
│  OrderBook (@heap) — too large for stack            │
└─────────────────────────────────────────────────────┘
```

## Running

```bash
# Type-check (validates all decorator constraints)
cargo run -p flux-cli -- check demos/advanced_decorators/strategy.flux

# Build to Rust (see generated #[repr] attributes)
cargo run -p flux-cli -- build demos/advanced_decorators/strategy.flux

# Backtest
cargo run -p flux-cli -- backtest demos/advanced_decorators/strategy.flux \
  --data demos/hello_world/sample_data.csv \
  --capital 100000
```

## Key Takeaways

1. **Decorators are zero-cost abstractions** — they compile to Rust attributes and intrinsics
2. **The type checker enforces compatibility** — invalid combinations are caught at compile time
3. **You don't need all decorators** — most strategies only need `@aligned` and `@immutable`
4. **Performance decorators are opt-in** — the default (`@stack` with `Clone, Copy`) is fast enough for most cases
5. **Decorators compose with the module system** — imported stdlib structs can be used alongside decorated user structs
