# FlowArrow Formatting

This document defines the canonical authoring format for FlowArrow source files
and the source transforms performed by `flowarrow fmt`.

The formatter is parse-driven: it first parses the program, then prints a
canonical representation of the parsed module. Formatting must not change the
program's data-dependency graph.

## Command

```text
flowarrow fmt <path.flow>
flowarrow fmt <path.flow> --check
flowarrow fmt <path.flow> --stdout
flowarrow fmt --stdin
```

- Without flags, `fmt` rewrites the file in place.
- `--check` exits successfully only when the file is already canonical.
- `--stdout` writes the canonical source to stdout and leaves the file
  unchanged.
- `--stdin` reads source from stdin and writes the canonical source to stdout
  for editor integrations and unsaved buffers.

## Canonical Source Shape

FlowArrow source is written as compact declaration groups separated by one
blank line. Consecutive imports form one group, consecutive type aliases form
one group, and callable declarations are separated from neighboring groups.
Every file ends with exactly one trailing newline.

```flow
import std.cli { Args }
import std.math { add }

program main(args: Args) -> exit_code: Int {
    (1, 2) -> add -> $exit_code
}
```

### Indentation

- Top-level declarations start at column 1.
- Callable bodies use four spaces.
- Tabs are never emitted.
- Closing braces of callable bodies start at column 1.

### Declarations

Imports and type aliases are grouped with declarations of the same kind.
Group transitions, nodes, and programs are separated from neighboring
declarations by one blank line.

```flow
type Pair = (Int, Real)

node add_pair(pair: Pair) -> out: Int {
    $pair -> add -> $out
}
```

Type aliases use one space around `=`.

### Imports

Alias imports stay on one line:

```flow
import std.math as math
```

Selective imports stay on one line when they are short:

```flow
import std.bytes { split_lines, concat_bytes }
```

Selective imports are expanded when the item count or line width makes the
inline form hard to scan:

```flow
import std.vector {
    sum,
    mean,
    add as vector_add,
    equals as vector_equals,
}
```

Multiline selective imports use one item per line, four-space indentation, and
a trailing comma on every item.

### Callable Headers

Input ports are written inside `(` and `)`, separated by `, `. Empty input
lists are written as `()`.

Single-output callables omit output parentheses:

```flow
node abs(value: Real) -> out: Real {
    $value -> scalar_abs -> $out
}
```

Multiple-output callables use a parenthesized output list:

```flow
node split(value: Pair) -> (left: Int, right: Real) {
    $value -> first -> $left
    $value -> second -> $right
}
```

### Types

Type punctuation is normalized:

- Port types use `name: Type`.
- Tuple and type-argument commas are followed by one space.
- Union alternatives use ` | `.
- Brackets stay tight: `Seq[Real]`, not `Seq [ Real ]`.

```flow
node f(input: (Seq[Real], Seq[Real])) -> out: Int | Real {
    $input -> g -> $out
}
```

### Chains

Every chain is printed on its own line. Arrows use one space on both sides
before alignment padding:

```flow
$input     -> split_lines      -> $raw_lines
$raw_lines -> filter not_empty -> $lines
$lines     -> map parse_real   -> $numbers
```

Adjacent chain lines are vertically aligned by arrow position. A blank line
resets the alignment group:

```flow
$input -> split_lines -> $raw_lines

$numbers -> reduce add(identity: 0.0) -> $total
$total   -> format_real               -> $total_bytes
```

Tuple joins and sequence literals use tight delimiters with `, ` between
items. Tuple binding targets use the same spacing:

```flow
($left, $right) -> add -> $sum
$pair -> ($left, $right)
[$sum, "\n"] -> concat_bytes -> $output
```

### Combinators

Combinators are printed in their compact canonical form:

```flow
$xs -> map parse_real -> $numbers
$xs -> filter not_empty -> $nonempty
$numbers -> reduce add(identity: 0.0) -> $total
$numbers -> scan add(identity: 0.0) -> $prefixes
$items -> repeat<$count> step -> $out
$lines -> fault map parse_real { ok -> $numbers, fault -> $faults }
```

`repeat` keeps the count tight inside angle brackets. `reduce` and `scan`
write `identity:` with one space after the colon.

### Literals

String literals are re-escaped with the compiler-supported escapes:
`\"`, `\\`, `\n`, `\t`, and `\r`.

Real literals remain real literals. For example, parsed `0.0` is printed as
`0.0`, not `0`, so formatting does not turn a real literal into an integer
literal.

### Comments

Standalone `#` comments are preserved and re-indented to the surrounding
canonical indentation. Common trailing chain comments are preserved with two
spaces before `#`.

```flow
# Explain the edge group.
($b, 123) -> eq -> $is_left_brace  # {
```

Standalone block comments are preserved and re-indented. Formatting comments
inside strings is never attempted.

## Transform Summary

`flowarrow fmt` applies these transforms:

- Parses the file and rejects invalid FlowArrow source.
- Removes non-canonical horizontal whitespace.
- Reprints declarations with canonical blank-line separation.
- Normalizes imports, including multiline expansion for long selective imports.
- Normalizes callable headers, port lists, output lists, and type punctuation.
- Reprints each chain with canonical arrow spacing and vertically aligned
  connected chain groups.
- Normalizes tuple, sequence, string, bool, int, and real literals.
- Removes hand alignment of arrows and comments that depends on neighboring
  line widths.
- Preserves standalone comments and common trailing comments while normalizing
  their indentation.
