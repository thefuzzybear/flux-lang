# =============================================================================
# Performance Types — SIMD, SoA, Streaming, Heap Patterns
# =============================================================================

# @simd(256) — SIMD-Width Alignment (AVX2)
# Aligns the struct for AVX2 (256-bit = 32-byte alignment). Ensures the
# struct can be loaded into SIMD registers without alignment faults.
# Valid widths: 128 (SSE), 256 (AVX2), 512 (AVX-512).
# Emits #[repr(align(32))] for 256-bit width.

@simd(256)
struct PriceVector {
    p0: f64,
    p1: f64,
    p2: f64,
    p3: f64
}

# @soa — Struct-of-Arrays Transformation
# Transforms array-of-structs into struct-of-arrays layout automatically.
# Use when iterating over one field across many instances (e.g., computing
# sum(prices) across 1000 ticks). SoA layout enables SIMD vectorization.
# Restriction: all fields must be scalar (f64, int, bool).

@soa
struct TickFeature {
    price_delta: f64,
    volume_ratio: f64,
    spread_normalized: f64,
    imbalance_score: f64
}

# @streaming — Non-Temporal Stores (Write-Once Optimization)
# Uses non-temporal store instructions that bypass the cache hierarchy.
# Use for data you won't read again soon (fill logs, audit trails).
# Prevents polluting L1/L2 cache with write-once data.

@streaming
struct FillLog {
    timestamp: f64,
    price: f64,
    size: f64,
    side: int,
    order_id: int
}

# @heap — Heap Allocation for Large Structs
# Allocates on the heap (Box<T>) instead of the stack. Passed by reference.
# Use when the struct is too large for the stack frame or when you need
# stable memory addresses for external references.
# Emits #[derive(Clone)] instead of #[derive(Clone, Copy)].

@heap
struct LargeBuffer {
    prices: [f64; 256],
    volumes: [f64; 256],
    count: int,
    capacity: int
}

# @stack — Explicit Stack Allocation (Default)
# Explicitly marks a struct as stack-allocated with Copy semantics.
# This is the default behavior (no decorator needed), but @stack makes
# the intent explicit in code reviews. Emits #[derive(Clone, Copy)].

@stack
struct QuoteUpdate {
    bid: f64,
    ask: f64,
    bid_size: f64,
    ask_size: f64
}
