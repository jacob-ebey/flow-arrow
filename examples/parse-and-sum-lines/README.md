# parse-and-sum-lines

```text
$ printf "1\n2\n3.5\n" | flowarrow run main.flow
6.5
```

## Why this example matters

This is the smallest useful program for designing FlowArrow's fault
handling. The happy path is intentionally simple: read newline-delimited
numbers from stdin, parse each line, sum the values, and print the total.

The important case is invalid input:

```text
$ printf "1\nwat\n3\n" | flowarrow run main.flow
```

Today, `parse_real` is specified as a host-runtime validation fault.
That is enough to reject bad input, but it does not answer the language
design questions this example is meant to force:

1. How does `parse_real` signal a graph-visible fault without becoming
   an exception or control-flow construct?
2. If `fault map parse_real` sees multiple invalid lines, does FlowArrow
   stop at the first fault or accumulate all line-level faults?
3. How does a fault preserve useful context such as line number and
   source bytes?
4. How does the program route diagnostics to `write_stderr` while routing
   successful output to `write_stdout`?
5. How is the final `exit_code` derived without hidden sequencing or
   implicit control flow?

Faults are for unintended invalid states, not ordinary control flow. A
program should not use the fault mechanism as a substitute for `select`,
`filter`, or other graph-visible dataflow.

This example uses the proposed partitioning syntax:

```flow
$lines -> fault map parse_real { ok -> $numbers, fault -> $faults }
```

The `ok` output is a `Seq[Real]` containing successfully parsed values.
The `fault` output is a `Seq[Fault]` containing diagnostics for invalid
elements.

## Desired eventual behavior

For input like:

```text
1
wat
3
```

the designed mechanism should be able to produce a diagnostic such as:

```text
line 2: expected Real, got "wat"
```

and return a non-zero exit code through explicit graph structure. For
valid input, the parse and reduce path remains parallel-friendly:
`fault map parse_real` can parse lines independently and `reduce
add(identity: 0.0)` can still lower to a balanced reduction tree.

## Syntax/design pressure

This example deliberately needs only one faultable operation:

```flow
$lines -> fault map parse_real { ok -> $numbers, fault -> $faults }
```

That keeps the design focused on the semantics of faultable pure nodes,
faultable `map`, diagnostic data, and effect-boundary reporting rather
than on CSV parsing, CLI flags, optional values, or recovery policies.
