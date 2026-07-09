# Flux Language — VS Code / Kiro Extension

Syntax highlighting for the Flux trading language.

## Features

- Full syntax highlighting for `.flux` files
- Keywords, control flow, struct types, decorators, built-in functions
- Type annotations (`f64`, `int`, `bool`, `str`)
- Struct type names (any capitalized identifier)
- Decorator syntax (`@aligned(64)`, `@packed`, etc.)
- Signal functions (`OPEN`, `CLOSE`, `CLOSE_QTY`)
- Bar context variables (`close`, `open`, `high`, `low`, `volume`, `symbol`, `in_position`)
- Comment highlighting (`# ...`)
- String and number literals

## Development Setup

For live development of the grammar (changes picked up on window reload):

```bash
# Replace the installed extension with a symlink to the source
rm -rf ~/.kiro/extensions/flux-lang.flux-lang-0.2.0
ln -s /path/to/flux-lang/editors/vscode ~/.kiro/extensions/flux-lang.flux-lang-0.2.0

# For VS Code instead of Kiro:
# rm -rf ~/.vscode/extensions/flux-lang.flux-lang-0.2.0
# ln -s /path/to/flux-lang/editors/vscode ~/.vscode/extensions/flux-lang.flux-lang-0.2.0
```

After symlinking, any edit to `syntaxes/flux.tmLanguage.json` takes effect on **Cmd+Shift+P → Reload Window**.

## Files

| File | Purpose |
|------|---------|
| `package.json` | Extension manifest (language ID, grammar path) |
| `syntaxes/flux.tmLanguage.json` | TextMate grammar (tokenization rules) |
| `language-configuration.json` | Bracket pairs, auto-close, comment toggling |

## Grammar Scopes

| Element | Scope | Typical Color |
|---------|-------|---------------|
| `struct`, `fn`, `strategy`, `from`, `import` | `keyword.other.flux` | Blue |
| `if`, `else`, `for`, `while`, `return` | `keyword.control.flux` | Purple |
| `Quote`, `Tick`, `Bar`, `Level` (capitalized) | `entity.name.type.struct.flux` | Green/Aqua |
| `f64`, `int`, `bool`, `str` | `support.type.primitive.flux` | Cyan |
| `@aligned`, `@packed`, `@volatile` | `meta.decorator.flux` | Yellow/Gold |
| `fn calc_spread` (definition) | `entity.name.function.flux` | Yellow |
| `calc_spread(q)` (call site) | `entity.name.function.call.flux` | Yellow |
| `OPEN`, `CLOSE`, `CLOSE_QTY` | `entity.name.function.signal.flux` | Magenta |
| `close`, `open`, `high`, `volume` | `variable.language.flux` | Red/Orange |
| `sma`, `ema`, `stddev`, `rsi` | `support.function.builtin.flux` | Cyan |
| `->` | `punctuation.arrow.flux` | Grey |
| `::` | `punctuation.separator.namespace.flux` | Grey |
| `true`, `false`, `null` | `constant.language.flux` | Orange |
| `42`, `3.14` | `constant.numeric.flux` | Orange |
| `"hello"` | `string.quoted.double.flux` | Green |
| `# comment` | `comment.line.number-sign.flux` | Grey |

## Troubleshooting

**Files show as "Plain Text":**
- Click the language mode in the bottom-right status bar → select "Flux"
- If "Flux" doesn't appear, the extension isn't installed. Check `~/.kiro/extensions/`

**Grammar changes not picked up:**
- Make sure you're using the symlink setup (see Development Setup above)
- Reload the window: Cmd+Shift+P → "Reload Window"

**"activationEvents should be omitted" warning:**
- This extension is grammar-only (no JavaScript). Don't add `activationEvents` to `package.json`.
