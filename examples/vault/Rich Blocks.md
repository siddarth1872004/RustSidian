# Rich blocks

Code with highlighting:

```rust
use std::collections::HashMap;

/// A tiny cache.
fn fib(n: u64) -> u64 {
    if n < 2 { return n; } // base case
    let name = "fib";
    fib(n - 1) + fib(n - 2)
}
```

```python
def greet(name: str) -> str:
    """Say hi."""
    return f"hi {name}" if name else None  # fallback
```

```json
{"name": "walnut", "tags": ["a", "b"], "n": 3.5, "ok": true}
```

```diff
@@ -1,2 +1,2 @@
-old line
+new line
```
