# `std.stream`

Generic stream type for values that should move through a graph without
being materialized as a `Seq` or `Bytes` value.

## Types

```text
Stream[T]
```

`Stream[T]` is an opaque runtime stream of values whose item type is
`T`. The stream type is generic: `Stream[Bytes]`, `Stream[Int]`, and
`Stream[(Int,Bytes)]` are distinct FlowArrow types.

File-backed byte stream nodes are boundary filesystem operations and are
exported from [`std.fs`](./fs.md).
