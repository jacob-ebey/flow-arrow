# json-parser

```text
$ printf '[1, 2.5, 3]' | flowarrow run main.flow
{"count":3,"sum":6.5}
```

A flat JSON array of numbers is parsed, then summarised as a JSON
object. Framing faults and per-element parse faults surface as
graph-visible diagnostics on stderr with a non-zero exit code.

```text
$ printf '[1, wat, 3]' | flowarrow run main.flow
line 2: expected f64, got "wat"
$ echo $?
65

$ printf '1, 2]' | flowarrow run main.flow
strip_prefix: input does not start with the expected prefix
$ echo $?
65
```

Whitespace tolerance is total: leading, trailing, and inter-element
whitespace are all accepted, and the empty array parses to zero
elements.

```text
$ printf '   [ 1 , 2 , 3.5 , -4 ]\n' | flowarrow run main.flow
{"count":4,"sum":2.5}

$ printf '[]' | flowarrow run main.flow
{"count":0,"sum":0}
```

## Why this exercises FlowArrow

A textbook recursive-descent JSON parser is not expressible in
FlowArrow: the language forbids recursion (§10 of `overview.md`) and
data-dependent graph topology (§6.0.1 of `syntax.md`), because both
would prevent the compiler from writing down the complete dependency
DAG ahead of time.

A *flat* JSON array of numbers is the largest JSON subset that fits
that constraint while still exercising real byte-level parsing. This
example demonstrates:

- byte-level boundary parsing of a structured format
- bracket framing via `strip_prefix` / `strip_suffix`, which produce
  graph-visible faults instead of exceptions or early returns
- comma-separated tokenisation via `split_on`
- per-element whitespace tolerance via `map trim` and `filter
  not_empty` (the filter makes `[]` parse to zero elements)
- parallel parsing with `fault map parse_real { ok, fault }`
- two independent parallel aggregations (`count` via `map one`
  followed by `reduce add`, and `sum` via `reduce add`) rooted at the
  same upstream sequence; both reductions lower to balanced trees and
  may execute concurrently
- pure dataflow routing of success vs. fault output via three
  `select` joins, with the exit code derived from `max` over both
  write statuses

## Dataflow shape

```text
() -> read_stdin -> trim                         -> $framed
($framed, "[")  -> strip_prefix                  -> $after_open
($after_open, "]") -> strip_suffix               -> $inner
($inner, ",") -> split_on                        -> $raw_tokens
$raw_tokens -> map trim                          -> $tokens
$tokens -> filter not_empty                      -> $nonempty
$nonempty -> fault map parse_real { ok -> $numbers, fault -> $faults }

$numbers -> map one                              -> $ones
$ones -> reduce add(identity: 0)                 -> $count
$numbers -> reduce add(identity: 0.0)            -> $sum
($count, $sum) -> render_summary                 -> $success_output

$faults -> has_faults                            -> $invalid_input
$faults -> format_faults                         -> $fault_output
($invalid_input, "",           $success_output) -> select -> $stdout_output
($invalid_input, $fault_output, "")             -> select -> $stderr_output
```

## Why this is *not* a full JSON parser

A complete JSON parser (RFC 8259) needs three capabilities FlowArrow
does not currently provide:

1. **Recursive structure.** Objects and arrays nest arbitrarily. The
   language forbids recursion, so a recursive descent parser is not
   expressible.
2. **Data-dependent topology.** A shift/reduce parser whose stack
   shape depends on input would change the graph at runtime, which
   §6.0.1 of `syntax.md` forbids.
3. **Richer byte primitives.** Strings need `\` escape decoding,
   numbers need richer lexing than this example bothers with, and
   objects need `:`-separated key/value handling. The new
   `trim` / `split_on` / `strip_prefix` / `strip_suffix` nodes in
   `std.bytes` are enough for flat numeric arrays but not for the
   full grammar.

A flat numeric array is the largest JSON subset that exercises real
byte-level parsing under FlowArrow's static-DAG constraints without
piling on stdlib primitives that wouldn't generalise.

## What it does *not* require

- No loops.
- No mutation.
- No recursion.
- No `if` / `else`; the success-vs-fault decision is three pure
  `select` joins.
- No statement ordering inside `main`; rearranging the chains
  produces an identical program.

## Related examples

- [`../parse-and-sum-lines`](../parse-and-sum-lines) — the same fault
  routing pattern over newline-delimited input instead of JSON framing.
- [`../add-numbers-from-stdin`](../add-numbers-from-stdin) — the
  fault-unaware version that lets `parse_real` faults propagate as
  `Faultable[i64]` through the program's exit code.
