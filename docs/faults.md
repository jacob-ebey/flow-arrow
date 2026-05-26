# FlowArrow faults

Faults are FlowArrow's mechanism for unintentional invalid states, such
as malformed input, numeric overflow, failed boundary I/O, or violated
data validation. They are not exceptions and must not be used for
ordinary control flow.

FlowArrow keeps the core rule:

```text
syntax = data dependency
```

A fault mechanism must therefore be graph-visible. Faults may affect
whether a program returns a successful exit code, but they must not hide
dynamic dispatch, secret branches, mutation, or statement ordering.

## Terminology

| Term | Meaning |
| --- | --- |
| Static error | Compile-time invalid FlowArrow source, such as a type mismatch or duplicate binding. |
| Fault | Runtime invalid data or boundary fault, such as `parse_real("wat")` or a stdout write fault. |
| Diagnostic | Data describing a fault for humans or tooling. |
| Faultable node | A node that can signal a fault for some inputs. |
| `Faultable[T]` | A value of type `T` whose producing graph may fault unless a handler consumes the fault path. |

Static errors are still compile-time errors. Runtime invalid states are
faults.

## Design constraints

1. Faults are unintended. Use `select`, `filter`, and explicit dataflow
   for expected choices.
2. Faults are visible to the graph. A program must be able to route
   diagnostics to `write_stderr` and derive a non-zero `exit_code`
   without hidden sequencing.
3. Pure nodes stay pure. A faultable pure node may signal invalid data,
   but it still cannot perform I/O, mutate state, observe time, or depend
   on thread identity.
4. Parallelism is preserved. Faultable `map` should define whether faults
   short-circuit or accumulate, but it must not introduce hidden
   element-to-element dependencies.
5. The C ABI must represent faults explicitly. Faults must not cross FFI
   boundaries via exceptions, `longjmp`, or host-language unwinding.

## Faultability propagation

Handling faults is optional. If a definition does not handle a faultable
operation, the faultability propagates through its output type:

```flow
program main(args: Args) -> exit_code: Faultable[i64] {
    () -> read_stdin -> parse_int -> $exit_code
}
```

That declaration is honest about the fact that malformed stdin can fault.
Plain values can also flow into matching faultable outputs. If a declaration or
match arm expects `Faultable[T]` and the produced value is `T`, the compiler
wraps it as the successful branch of the faultable value.

If a definition handles the fault path, it may return a non-faultable
type:

```flow
$lines -> fault map parse_real { ok -> $numbers, fault -> $faults }
```

`fault map` partitions a `Seq[Faultable[T]]` into `$numbers : Seq[T]` and
`$faults : Seq[Fault]`. The fault path is ordinary dataflow: it can be
formatted, routed to `write_stderr`, and used to derive the final
`$exit_code`.

## Arithmetic faults

Fixed-width `i64` arithmetic reports overflow as recoverable data:

```text
add_i64 : (i64, i64) -> Faultable[i64]
sub_i64 : (i64, i64) -> Faultable[i64]
mul_i64 : (i64, i64) -> Faultable[i64]
neg_i64 : i64        -> Faultable[i64]
abs_i64 : i64        -> Faultable[i64]
```

`f64` `add_f64`, `sub_f64`, `mul_f64`, `neg_f64`, and `abs_f64` remain
plain `f64` operations. Division, remainder, and `sqrt` are faultable because
they have invalid inputs such as zero divisors or negative square roots.

Integer `add_i32`, `add_i64`, `sub_i32`, `sub_i64`, `mul_i32`, `mul_i64`,
`neg_i32`, `neg_i64`, `abs_i32`, and `abs_i64` return `Faultable[...]`
for fixed-width integer types because overflow is recoverable graph data.
For example, `reduce add_i64(identity: 0)` over `Seq[i64]` returns
`Faultable[i64]`, and `scan add_i64(identity: 0)` over `Seq[i64]` returns
`Seq[Faultable[i64]]`. Use
`std.fault.expect` only at a boundary where aborting on a fault is the
intended policy. Use `fault map` or explicit `Faultable[...]` outputs when
the graph should recover and continue.

## Seed example

`examples/parse-and-sum-lines/` is the minimal design case:

```flow
$lines -> fault map parse_real { ok -> $numbers, fault -> $faults }
$numbers -> reduce add_f64(identity: 0.0) -> $total
```

The design question this example now exercises is how `parse_real`
reports malformed input inside a parallel `fault map` while preserving
useful context such as line number and source bytes.

For valid input, the graph should keep the same parallel shape. For
invalid input, the program should be able to produce diagnostics such as:

```text
line 2: expected f64, got "wat"
```

and return a non-zero exit code through explicit graph structure.

## Open questions

1. Should `Faultable[T]` remain a type constructor, or should
   faultability become a distinct function effect tracked by the
   typechecker?
2. Should `fault map` always accumulate all element faults, or should
   accumulation vs. first-fault be explicit policy?
3. What is the stable diagnostic data model?
4. How are stdout/stderr boundary faults combined with data-validation
   faults to produce a program exit code?
5. Which stdlib nodes are faultable in the initial profile?
