# `std.seq`

Generic sequence helpers used by higher-level source-backed stdlib
modules.

| Export | Input | Output | Description |
| --- | --- | --- | --- |
| `length` | `Seq[V]` | `Int` | Sequence length |
| `zip` | `(Seq[A],Seq[B])` | `Seq[(A,B)]` | Pair equal-length sequences by position |
| `broadcast_left` | `(A,Seq[B])` | `Seq[(A,B)]` | Pair one left value with each item in a sequence |
| `broadcast_right` | `(Seq[A],B)` | `Seq[(A,B)]` | Pair each sequence item with one right value |
| `transpose` | `Seq[Seq[V]]` | `Seq[Seq[V]]` | Transpose a rectangular nested sequence |
| `flatten` | `Seq[Seq[V]]` | `Seq[V]` | Concatenate nested rows in order |
| `inner_length` | `Seq[Seq[V]]` | `Int` | First inner sequence length, or `0` for empty input |
| `group_by_id` | `(Seq[V],Seq[Int])` | `Seq[Seq[V]]` | Group values by non-decreasing integer ids |
| `shift_right` | `(Seq[V],V)` | `Seq[V]` | Shift sequence right, inserting an initial value |
| `head` | `Seq[V]` | `Faultable[V]` | First item, or a fault for empty input |

`zip` faults when sequence lengths differ. `transpose` faults when inner
sequence lengths differ. `flatten` does not require rectangular input.

