# `std.stream`

Generic stream type for values that should move through a graph without
being materialized as a `Seq` or `Bytes` value.

## Types

```text
Stream[T]
```

## Nodes

```text
to_seq : Stream[V] -> Faultable[Seq[V]]
drain  : Stream[V] -> Faultable[Int]
```

`Stream[T]` is an opaque runtime stream of values whose item type is
`T`. The stream type is generic: `Stream[Bytes]`, `Stream[Int]`, and
`Stream[(Int,Bytes)]` are distinct FlowArrow types.

File-backed byte stream nodes are boundary filesystem operations and are
exported from [`std.fs`](./fs.md).

`to_seq` consumes a pull-readable stream and materializes all values into a
sequence. `drain` consumes a pull-readable stream without materializing values.
Both nodes close the stream after EOF or on fault. Streams produced for HTTP
serving are not pull-readable and must be passed to their serving boundary.
