# Contributing to Flux

Thank you for your interest in contributing to Flux. This document provides guidelines for contributing.

## Getting Started

### Prerequisites

- Rust 1.75 or later
- Git
- Basic understanding of compilers (helpful but not required)

### Setup

```bash
# Clone the repository
git clone https://github.com/thefuzzybear/flux-lang.git
cd flux-lang

# Build the project
cargo build

# Run tests
cargo test

# Run clippy
cargo clippy

# Format code
cargo fmt
```

## Before You Start

**Read these documents first:**
1. `.claude/CLAUDE.md` - Repository context and architecture
2. `CODING_STANDARDS.md` - Coding conventions
3. `docs/architecture/00-overview.md` - System architecture

**Choose an issue:**
- Look for issues labeled `good-first-issue`
- Comment on the issue to claim it
- Ask questions if anything is unclear

## Development Workflow

1. **Create a branch**
   ```bash
   git checkout -b feat/your-feature-name
   ```

2. **Make changes**
   - Follow coding standards in `CODING_STANDARDS.md`
   - Write tests for new features
   - Update documentation

3. **Test your changes**
   ```bash
   cargo test
   cargo clippy
   cargo fmt --check
   ```

4. **Commit**
   ```bash
   git add .
   git commit -m "feat(scope): description

   Detailed explanation of changes."
   ```

5. **Push and create PR**
   ```bash
   git push origin feat/your-feature-name
   ```
   Then open a Pull Request on GitHub.

## Commit Message Format

```
<type>(<scope>): <subject>

<body>
```

**Types:**
- `feat` - New feature
- `fix` - Bug fix
- `docs` - Documentation
- `refactor` - Code refactoring
- `perf` - Performance improvement
- `test` - Tests
- `chore` - Maintenance

**Scopes:**
- `lexer`, `parser`, `typeck`, `codegen`, `runtime`, `cli`, `docs`

## Code Review Process

1. All PRs require review
2. Address review comments
3. Keep PR focused (one feature/fix per PR)
4. Squash commits before merge

## Testing

- Write unit tests for new features
- Add integration tests for end-to-end functionality
- Use property tests for invariants
- Ensure all tests pass before PR

## Documentation

- Update relevant docs when adding features
- Add examples for user-facing features
- Keep CLAUDE.md updated for agents
- Write doc comments for public API

## Questions?

- Open an issue with the `question` label
- Join Discord (link in README)
- Check existing documentation first

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
