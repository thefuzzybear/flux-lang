# Integration test: struct with type errors for diagnostics testing
# This file intentionally contains errors to verify error reporting.

struct Point {
    x: f64,
    y: f64
}

fn use_point(p: Point) -> f64 {
    return p.x + p.y
}

strategy StructErrorStrategy {
    params {
        threshold = 1.0
    }

    state {
        count = 0
    }

    on bar {
        # Error: accessing a field that doesn't exist on Point
        p = Point { x = 1.0, y = 2.0 }
        bad = p.z
    }
}
