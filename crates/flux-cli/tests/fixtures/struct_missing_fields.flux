# Integration test: struct literal with missing fields
# This file intentionally contains a missing-fields error.

struct Config {
    threshold: f64,
    window: int,
    active: bool
}

strategy MissingFieldsStrategy {
    params {
        size = 100.0
    }

    state {
        count = 0
    }

    on bar {
        # Error: missing 'active' field in struct literal
        c = Config { threshold = 1.5, window = 20 }
        x = c.threshold
    }
}
