# FlowArrow examples

Small programs intended to stress the language design.

Every example assumes the standard library provides a small set of
pure dataflow nodes for parsing, formatting, and string manipulation:

```text
# Byte / text
split_lines       : Bytes -> Seq[Bytes]
parse_real        : Bytes -> Real
parse_int         : Bytes -> Int
format_int        : Int   -> Bytes
format_real       : Real  -> Bytes
concat_bytes      : Seq[Bytes] -> Bytes              # associative; identity: ""
join_bytes        : (Seq[Bytes], Bytes) -> Bytes     # second arg is separator

# Predicates / arithmetic
not_empty         : Bytes -> Bool
add               : (Real, Real) -> Real             # associative
add_int           : (Int, Int)   -> Int              # associative
sub_int           : (Int, Int)   -> Int
```

These are stdlib primitives, not language built-ins; the examples
work as soon as a stdlib provides them.

| Example                       | What it shows                                          |
| ----------------------------- | ------------------------------------------------------ |
| `add-numbers-from-stdin/`     | Boundary I/O, dynamic-size sequences, parallel reduce. |
| `99-bottles/`                 | Pure string generation via `range_step` + `map` + concat reduce. |
