# `std.seq`

Generic sequence helpers used by higher-level source-backed stdlib
modules.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `length` | `Seq[V]` | `Int` | Sequence length |
| `is_empty` | `Seq[V]` | `Bool` | Whether the sequence has no items |
| `zip` | `(Seq[A],Seq[B])` | `Seq[(A,B)]` | Pair equal-length sequences by position |
| `broadcast_left` | `(A,Seq[B])` | `Seq[(A,B)]` | Pair one left value with each item in a sequence |
| `broadcast_right` | `(Seq[A],B)` | `Seq[(A,B)]` | Pair each sequence item with one right value |
| `transpose` | `Seq[Seq[V]]` | `Seq[Seq[V]]` | Transpose a rectangular nested sequence |
| `flatten` | `Seq[Seq[V]]` | `Seq[V]` | Concatenate nested rows in order |
| `inner_length` | `Seq[Seq[V]]` | `Int` | First inner sequence length, or `0` for empty input |
| `group_by_id` | `(Seq[V],Seq[Int])` | `Seq[Seq[V]]` | Group values by non-decreasing integer ids |
| `shift_right` | `(Seq[V],V)` | `Seq[V]` | Shift sequence right, inserting an initial value |
| `head` | `Seq[V]` | `Faultable[V]` | First item, or a fault for empty input |
| `tail` | `Seq[V]` | `Seq[V]` | All items after the first item |
| `reverse` | `Seq[V]` | `Seq[V]` | Reverse item order |
| `take` | `(Seq[V],Int)` | `Seq[V]` | First `n` items, clamped to sequence length |
| `drop` | `(Seq[V],Int)` | `Seq[V]` | Items after the first `n`, clamped to sequence length |
| `fill` | `(V,Int)` | `Seq[V]` | A sequence containing one value repeated `n` times |
| `slice` | `(Seq[V],Int,Int)` | `Seq[V]` | Half-open index range |
| `last` | `Seq[V]` | `Faultable[V]` | Last item, or a fault for empty input |
| `get` | `(Seq[V],Int)` | `V` | Item at index, or a usage fault |
| `get_or` | `(Seq[V],Int,V)` | `V` | Item at index, or fallback when out of range |
| `at` | `(Seq[V],Int)` | `Faultable[V]` | Item at index, or a fault |
| `append` | `(Seq[V],V)` | `Seq[V]` | Append one item |
| `set` | `(Seq[V],Int,V)` | `Seq[V]` | Return a copy with one item replaced |
| `concat` | `(Seq[V],Seq[V])` | `Seq[V]` | Concatenate two sequences |

`zip` faults when sequence lengths differ. `transpose` faults when inner
sequence lengths differ. `flatten` does not require rectangular input.
`take` and `drop` reject negative counts. `fill` rejects negative counts.
