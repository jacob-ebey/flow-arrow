# `std.cli`

Types and pure helpers for command-line arguments and flags.

The process entry point receives CLI data as ordinary program input:

```flow
import std.cli { Args }

program main(args: Args) -> exit_code: Int {
    ...
}
```

`Args` is supplied by the host runtime. Inspecting it is pure dataflow;
reading stdin and writing stdout/stderr are separate explicit boundary
effects in [`std.io`](./io.md).

## Types

```text
Args
```

## Nodes

```text
argv         : Args -> Seq[Bytes]
flag_present : (Args, Bytes) -> Bool
flag_value   : (Args, Bytes) -> Bytes
```

## Semantics

### `argv`

Returns positional command-line arguments as bytes.

- `argv(args)[0]` is the first user argument, not the executable path.
- Argument decoding is left to explicit parsing nodes; `argv` returns
  bytes so programs do not implicitly depend on host string encoding.

### `flag_present`

Returns whether a flag is present.

- The second input is the flag name as bytes, for example `"--verbose"`.
- The initial profile recognises exact flag names only; no abbreviation
  or combined short-flag expansion is implied.

### `flag_value`

Returns a flag's value as bytes.

- The second input is the flag name as bytes, for example `"--output"`.
- If the flag is missing or has no value, behavior is a validation fault
  reported by the host runtime. A future optional-value type may make
  this total.

## Example

```flow
import std.cli { Args, argv, flag_present }

program main(args: Args) -> exit_code: Int {
    args -> argv -> positional
    (args, "--verbose") -> flag_present -> verbose
    0 -> id -> exit_code
}
```
