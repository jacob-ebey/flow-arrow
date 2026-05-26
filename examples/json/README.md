# json

Generic JSON tokenizer written in FlowArrow.

Reads JSON from stdin, emits a flat SAX-style token stream to stdout. Each
output line is `<depth> <KIND> <text>`, where `<depth>` is the nesting depth
*after* the token, `<KIND>` is one of `OBJECT_START`, `OBJECT_END`,
`ARRAY_START`, `ARRAY_END`, `COMMA`, `COLON`, `STRING`, `NUMBER`, `IDENT`,
and `<text>` is the verbatim source bytes that make up the token (so
`STRING` text retains its surrounding quotes and any backslash escapes).

Whitespace bytes are recognised by the tokenizer but discarded from the
emitted stream.

## Why is this interesting?

FlowArrow is a pure dataflow language: there is no recursion, no
data-dependent control flow, no sum types. A textbook JSON parser is
recursive over a tree, which would be impossible to express directly.

This example instead implements a **parallel, scan-based, SIMD-style
tokenizer**:

1. Each input byte is mapped to a 4-state transition table encoded as an
   `i64`. The 4 states are `OUT`, `STR` (inside a string), `ESC` (just past
   a backslash inside a string), and `RUN` (inside a number or literal name
   like `true`/`false`/`null`).
2. `scan compose_trans` fuses per-byte transition tables into per-position
   transitions-from-start. Because function composition is associative,
   this is a valid associative scan.
3. From the composed transition each byte's incoming state is recovered
   and combined with the byte itself to assign a token kind (or "not a
   start") to that position.
4. A prefix-sum of the start flags assigns each byte a token id;
   `group_by_id` collapses byte codes into per-token byte groups.
5. Whitespace tokens are filtered out, depth deltas are prefix-summed, and
   the result is formatted line-by-line.

The whole tokenizer is therefore a **straight-line dataflow graph** of
`map`/`scan`/`zip`/`group_by_id`/`filter` stages with no recursion or
conditional control flow.

## Example

Input:

```json
{"name":"json","matrix":[[1,2],[3,4]]}
```

Output:

```
1 OBJECT_START {
1 STRING "name"
1 COLON :
1 STRING "json"
1 COMMA ,
1 STRING "matrix"
1 COLON :
2 ARRAY_START [
3 ARRAY_START [
3 NUMBER 1
3 COMMA ,
3 NUMBER 2
2 ARRAY_END ]
2 COMMA ,
3 ARRAY_START [
3 NUMBER 3
3 COMMA ,
3 NUMBER 4
2 ARRAY_END ]
1 ARRAY_END ]
0 OBJECT_END }
```

## Running

```sh
echo '{"a":1,"b":[true,null]}' | flowarrow run examples/json/main.flow
```

## Limitations

This is a tokenizer, not a full validator. It does not verify that
brackets match, that numbers are well-formed, or that identifiers are
exactly `true`/`false`/`null`. The output stream is purely lexical.
