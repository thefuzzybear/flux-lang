# =============================================================================
# Order Types — Pool-Allocated, Bit-Packed Order Management
# =============================================================================

# @pool(256) — Pre-Allocated Object Pool / Slab Allocator
# Pre-allocates 256 instances in a contiguous slab with O(1) alloc/free.
# Perfect for order lifecycle: create → fill/cancel → recycle.
# Zero allocator overhead, zero fragmentation, deterministic latency.
# Reports pool-exhausted at runtime if all 256 slots are occupied.

@pool(256)
struct LiveOrder {
    price: f64,
    size: f64,
    remaining: f64,
    side: int,
    status: int
}

# @bitfield — Bit-Level Packing for Dense Flag Storage
# Packs bool (1 bit) and int(N) (N bits) fields into minimal bytes.
# Use when storing arrays of thousands of flag structs where every byte
# counts. Fields are accessed via bitwise shift/mask operations.
# Total bit count must not exceed 64 (fits in a single u64).

@bitfield
struct OrderFlags {
    is_active: bool,
    is_filled: bool,
    is_cancelled: bool,
    side: int(2),
    priority: int(4),
    venue_id: int(6)
}

# @packed — Zero Padding, Minimal Memory Footprint
# Removes all field alignment padding. Fields are laid out contiguously.
# Use for wire-format messages, historical tick storage, or anywhere
# memory footprint matters more than access speed.
# Emits #[repr(packed)] — field access uses safe unaligned reads.

@packed
struct TradeRecord {
    price: f64,
    size: f64,
    side: int,
    sequence: int
}
