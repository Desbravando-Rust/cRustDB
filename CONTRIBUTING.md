# Contributing

## Tests: AAA

Every `#[test]` follows the Arrange-Act-Assert layout:

- **Arrange**: build inputs and fixtures.
- **Act**: call the code under test, once.
- **Assert**: check the result.

Rules:

- Mark each section with `// Arrange`, `// Act`, `// Assert`, in that order, separated by a blank line.
- No setup? Omit `// Arrange`.
- Action and check inseparable (table loops `for (input, expected) in [...]`, a one-line call inside an assert)? Use a single `// Act & Assert`.
- When the action hides inside `assert_eq!(f(x), y)` and extracting it reads naturally, write `let got = f(x);` then `assert_eq!(got, y);`.

```rust
#[test]
fn splits_two_statements() {
    // Arrange
    let sql = "select 1; select 2";

    // Act
    let got = split(sql);

    // Assert
    assert_eq!(got.len(), 2);
}
```

### Validator (proposal)

Not wired into any gate yet. A heuristic check: every `#[test]`/`#[tokio::test]` body must contain `// Assert` or `// Act & Assert`. It prints `file: test` for each offender and exits 1:

```sh
awk '
/#\[(tokio::)?test/ { t = 1; name = ""; ok = 0; next }
t && name == "" && /fn [a-z_0-9]+/ {
    match($0, /fn [a-z_0-9]+/); name = substr($0, RSTART + 3, RLENGTH - 3)
    close_re = "^" substr($0, 1, match($0, /[^ ]/) - 1) "}$"; next
}
t && name != "" && /\/\/ (Assert|Act & Assert)$/ { ok = 1 }
t && name != "" && $0 ~ close_re { if (!ok) { print FILENAME ": " name; bad = 1 } t = 0 }
END { exit bad }
' $(grep -rlE '#\[(tokio::)?test' crates)
```

It could run from `.githooks/pre-commit` or `make lint` once every test file follows the layout. Known gap: files at the 500-line limit (they need splitting first).
