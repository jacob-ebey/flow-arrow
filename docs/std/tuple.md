# `std.tuple`

Generic tuple helpers for source-backed stdlib modules and user code.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `first` | `(A,B)` | `A` | Return the first item |
| `second` | `(A,B)` | `B` | Return the second item |
| `swap` | `(A,B)` | `(B,A)` | Swap a pair |

These helpers are pure data movement nodes. They do not evaluate either
side conditionally.
