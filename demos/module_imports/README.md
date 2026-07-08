# Module Imports Demo

A focused demonstration of Flux's cross-file import system using `::` path separators.
Shows how to organize trading logic across multiple files with layered modules.

## Project Structure

```
demos/module_imports/
├── strategy.flux               # Main strategy (entry point)
├── data.csv                    # Sample AAPL daily data
├── signals/
│   ├── entry.flux              # Entry signal logic (imports from math::stats)
│   ├── exit.flux               # Exit signal logic (self-contained)
│   └── math/
│       └── stats.flux          # Shared math helpers (z-score, ratios)
└── README.md
```

## How It Works

The main strategy imports high-level signal functions:

```flux
from signals::entry import {should_enter}
from signals::exit import {should_exit}
```

The entry module imports shared math helpers from a nested library:

```flux
# Inside signals/entry.flux
from math::stats import {z_score, safe_ratio, exceeds}
```

This `from math::stats` resolves **relative to the importing file's directory** —
so `signals/entry.flux` finds `signals/math/stats.flux`.

## Key Concepts Demonstrated

1. **`::` path separators** — `from signals::entry import {should_enter}`
2. **Library files** — `.flux` files with only `fn` definitions, no strategy block
3. **Transitive imports** — `entry.flux` imports from `math::stats.flux`, and the
   resolver pulls in the transitive dependencies automatically
4. **Selective inclusion** — Only `should_enter` is imported, but `z_score`,
   `safe_ratio`, and `exceeds` are pulled in transitively because `should_enter`
   calls `volume_confirmed` which calls them
5. **Relative path resolution** — Each library file resolves its own imports
   relative to its own directory, not the main file's

## Running

```bash
# Type-check the multi-file project
cargo run -p flux-cli -- check demos/module_imports/strategy.flux

# Backtest with sample data
cargo run -p flux-cli -- backtest demos/module_imports/strategy.flux \
  --data demos/module_imports/data.csv \
  --capital 50000
```

## Import Resolution Walkthrough

When you run `flux check strategy.flux`, here's what happens:

1. Parser sees `from signals::entry import {should_enter}` → file-module import
2. Resolver maps `signals::entry` → `./signals/entry.flux`
3. Parses `signals/entry.flux`, sees `from math::stats import {z_score, safe_ratio, exceeds}`
4. Resolves `math::stats` **relative to `signals/`** → `./signals/math/stats.flux`
5. Parses `signals/math/stats.flux`, extracts `z_score`, `safe_ratio`, `exceeds`
6. Back in `entry.flux`: selects `should_enter` + its transitive deps (`volume_confirmed`, `z_score`, `safe_ratio`, `exceeds`)
7. Merges all selected functions into the main program AST
8. Repeats for `signals::exit` → `./signals/exit.flux` → extracts `should_exit`
9. Typechecker sees the fully-merged program (as if everything were in one file)
