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
