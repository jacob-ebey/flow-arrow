## Language: **FlowArrow**

**Design rule:** every syntactic construct must denote a pure data-dependency graph.  
There is no statement ordering, mutation, hidden control flow, locks, blocking I/O, exceptions, or implicit global state.

`->` means **“this value is available to this computation/input.”**

FlowArrow defines “optimal” as:

> The compiler can always derive the complete dependency DAG, expose all legal parallelism, and schedule with minimum possible span under an explicit cost model.

---

# 1. Core syntax

```flow
node name(input1: Type, input2: Type) -> output: Type {
    $input1 -> opA -> $a
    $input2 -> opB -> $b
    ($a, $b) -> opC -> $output
}
```

A block is **not sequential**. Lines merely declare graph edges.

This:

```flow
$x -> f -> $a
$x -> g -> $b
($a, $b) -> h -> $y
```

means:

```text
         ┌─> f ─> $a ─┐
$x ──────┤            ├─> h ─> $y
         └─> g ─> $b ─┘
```

`f` and `g` must execute in parallel if resources exist.

---

# 2. Values are single-assignment

```flow
$x -> square -> $sx
```

binds `$sx` exactly once.

Illegal:

```flow
$x -> square -> $y
$z -> sqrt -> $y       # illegal: y already defined
```

There is no reassignment.

---

# 3. Fan-out

```flow
$x -> {
    normalize -> $n,
    histogram -> $h,
    edges     -> $e
}
```

Equivalent to three independent outgoing edges from `$x`.

No branch is ordered before another.

---

# 4. Join

```flow
($a, $b, $c) -> combine -> $result
```

`combine` waits only for `$a`, `$b`, and `$c`.

No other dependency exists.

---

# 5. Named ports

For multi-input operations, ports may be explicit:

```flow
$image  -> convolve.image
$kernel -> convolve.kernel
convolve.out -> $blurred
```

Equivalent shorthand:

```flow
($image, $kernel) -> convolve -> $blurred
```

Named ports are useful when order would be unclear.

---

# 6. Example: hypotenuse

```flow
node hypot(x: Real, y: Real) -> r: Real {
    $x -> square -> $xx
    $y -> square -> $yy
    ($xx, $yy) -> add -> sqrt -> $r
}
```

The compiler sees:

```text
x ─> square ─> xx ─┐
                   ├─> add ─> sqrt ─> r
y ─> square ─> yy ─┘
```

The two `square` operations are independent and parallel.

---

# 7. No ordinary `if`

FlowArrow does **not** have:

```flow
if condition {
    ...
} else {
    ...
}
```

because ordinary branching hides scheduling choices.

Instead, it has pure data selection:

```flow
$x -> neg -> $nx
$x -> is_positive -> $p
($p, $x, $nx) -> select -> $absx
```

`select` is a pure dataflow node:

```text
select(predicate, when_true, when_false)
```

Both candidate values are ordinary graph inputs. The compiler sees the full dependency graph.

FlowArrow also has `match` for static alternatives with runtime-selected
evaluation:

```flow
$req -> match {
    route("GET", "/health") -> health_response
    route("POST", "/echo")  -> echo_response
    _                       -> not_found
} -> $response
```

Every guard and arm node is visible to the compiler. The upstream value is
implicitly passed to each guard and selected arm node. Guards are pure `Bool`
nodes evaluated top-to-bottom, and only the selected arm node is evaluated.
`match` therefore introduces a control dependency without allowing dynamic
topology.

---

# 8. Data-parallel collections

Collections are first-class, but only through parallel-safe combinators.

## `map`

```flow
$xs -> map square -> $ys
```

Means each element is independent:

```text
ys[i] = square(xs[i])
```

No element may observe or mutate another.

---

## `zip`

```flow
($xs, $ys) -> zip -> $pairs
$pairs -> map multiply_pair -> $products
```

---

## `reduce`

Reductions require an associative operation and identity.

```flow
node dot(xs: Vec[N, Real], ys: Vec[N, Real]) -> s: Real {
    ($xs, $ys) -> zip
             -> map multiply_pair
             -> reduce add(identity: 0)
             -> $s
}
```

`reduce` is compiled as a balanced tree.

Illegal:

```flow
$xs -> reduce subtract(identity: 0) -> $y
```

because subtraction is not associative.

---

## `scan`

Prefix operations are allowed only for associative operators:

```flow
$xs -> scan add(identity: 0) -> $prefix_sums
```

The compiler emits a parallel prefix tree.

---

## Dynamic sizes

Collection lengths may be runtime values. The *shape* of the dataflow
graph stays static; only the *width* of parallel regions varies.

`range` produces a sequence of integers from a runtime length:

```flow
$n -> range -> $idxs                    # $idxs : Seq[Int], length = $n
$idxs -> map compute_pixel -> $pixels
$pixels -> reduce add(identity: 0) -> $total
```

`range_between` and `range_step` produce sequences from `(start, stop)`
and `(start, stop, step)` respectively.

`filter` produces a sequence whose length is data-dependent. It is
compiled as a parallel predicate evaluation followed by a parallel
prefix-sum compaction — topology stays static:

```flow
$xs -> filter is_positive -> $positives
```

`length` reports the runtime length of a sequence:

```flow
$xs -> length -> $n
```

`grid<...>` dimensions and `repeat<...>` counts may also be runtime
values (see §9 and §15).

### What stays forbidden

Runtime values may parameterise the **size** of a parallel region or
the **count** of iterations. They may **not** select which nodes exist
or how they are connected:

```flow
# illegal — operator is chosen at runtime
$op_name -> lookup_op -> $op
$xs -> map $op -> $ys

# illegal — sequential element-to-element dependency
$xs -> take_while is_positive -> $prefix
```

The full forbidden list lives in `syntax.md` §8.

---

# 9. Bounded iteration only

FlowArrow has no unbounded `while`.

Allowed with a compile-time count (the body unrolls into a fixed
dataflow chain):

```flow
$state -> repeat<10> step -> $final
```

Allowed with a runtime count (the body is a single static graph; the
runtime executes it `n` times):

```flow
($state, $n) -> repeat<$n> step -> $final
```

In both cases the *graph shape* is static. Only the *number of
unrollings* may vary at runtime.

Illegal:

```flow
while not_done {
    $state -> step -> $state
}
```

because the dependency graph is not statically bounded — termination
depends on a value computed inside the loop.

---

# 10. Recursion is forbidden

Illegal:

```flow
node fib(n: Int) -> r: Int {
    ...
    $n1 -> fib -> $a
    $n2 -> fib -> $b
    ...
}
```

General recursion creates dynamic, data-dependent graphs and cannot always be compiled into a statically optimal parallel schedule.

Use explicit parallel combinators instead.

---

# 11. Pure node definitions

Every node is pure.

```flow
node blur(img: Image[H, W], kernel: Kernel[3, 3]) -> out: Image[H, W] {
    ($img, $kernel) -> stencil2d radius<1> convolve_pixel -> $out
}
```

A node may not:

- mutate memory
- read global variables
- write files
- print
- allocate shared mutable objects
- acquire locks
- throw exceptions
- perform blocking I/O
- depend on time, randomness, or thread identity

Randomness must be explicit:

```flow
($seed, $index) -> random_uniform -> $value
```

Time must be explicit:

```flow
$timestamp -> compute_deadline -> $deadline
```

---

# 12. Effects are boundary-only

A complete program receives command-line arguments and flags as its
ordinary input and returns an integer process exit code.

```flow
import std.cli { Args }
import std.io { read_stdin, write_stdout }

program main(args: Args) -> exit_code: Int {
    () -> read_stdin -> $input
    $input -> parse -> transform -> encode -> $output
    $output -> write_stdout -> $exit_code
}
```

Standard input, standard output, standard error, and file I/O are
accessed through explicit boundary nodes such as `std.io` and `std.fs`,
not through `main`'s parameters or return value. These nodes are visible
in the dependency graph. A reusable `node` that calls a boundary node is
effectful by composition, so it can be called from a program but cannot
be used as a `map`, `filter`, `reduce`, or similar higher-order
function.

Runtime invalid states are **faults**, not exceptions and not ordinary
control flow. Faults are reserved for unintended invalid states such as
malformed input, numeric overflow, or boundary I/O faults. See
[`faults.md`](./faults.md).

---

# 13. Imports and the standard library

FlowArrow source files may import pure nodes, boundary nodes, and types
from the standard library, package dependencies, or local `.flow` files.
Imports are compile-time name resolution only: they do not create graph
nodes, edges, ordering, effects, or hidden global state.

```flow
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }
import std.real { parse_real, format_real }
import std.math as math
import "./filters.flow" { blur, sobel as detect_edges }
import "./format.flow" as format
```

There are two import sources:

- **Library imports** use a dotted module path (`std.bytes`,
  `std.math`, `acme.image.filters`). The `std` root is reserved for the
  FlowArrow standard library.
- **Local imports** use a string path resolved relative to the importing
  file (`"./filters.flow"`, `"../shared/math.flow"`).

There are two import forms:

```flow
# selective import: introduces bare names
import std.bytes { split_lines, concat_bytes }

# qualified import: introduces an alias namespace
import std.math as math
```

Selective imports make imported declarations available as ordinary
names:

```flow
$input -> split_lines -> $lines
```

Qualified imports require the alias:

```flow
($x, $y) -> math.add -> $sum
```

The `math.add` reference is still a statically resolved computation
node. It is not dynamic dispatch; the compiler resolves the alias and
target before building the dependency DAG.

Local imports work the same way:

```flow
import "./image/filters.flow" as filters

node enhance(img: Image[H, W, RGB]) -> out: Image[H, W, RGB] {
    $img -> filters.normalize -> $n
    $n -> filters.sharpen -> $out
}
```

Name collisions are compile-time errors unless one side is renamed:

```flow
import std.bytes { concat_bytes }
import "./html.flow" { concat_bytes as concat_html }
```

Import resolution is deterministic and acyclic. Cyclic imports are
ill-formed; imports are a module-system feature, not a way to construct
recursive or dynamic dataflow graphs.

All top-level `node` declarations are exportable from their source
module. A `program` declaration is an entry point, not a reusable pure
node, and cannot be called from another graph.

The examples in `examples/` import stdlib primitives such as
`read_stdin`, `write_stdout`, `split_lines`, `parse_real`,
`concat_bytes`, and `add` explicitly from the relevant `std.*` modules
instead of assuming they are globally available.

The initial standard-library module surface is documented in
[`docs/std/`](./std/).

---

# 14. Image-processing example

```flow
node detect_edges(img: Image[H, W, RGB]) -> edges: Image[H, W, Gray] {
    $img -> {
        grayscale -> $gray,
        histogram -> $hist
    }

    ($gray, $hist) -> equalize -> $eq
    $eq -> gaussian_blur radius<2> -> $smooth
    $smooth -> sobel -> $edges
}
```

The compiler sees:

```text
              ┌─> grayscale ─> $gray ─┐
$img ─────────┤                        ├─> equalize ─> gaussian_blur ─> sobel ─> $edges
              └─> histogram ─> $hist ──┘
```

`grayscale` and `histogram` are independent.

---

# 15. Matrix multiplication example

```flow
node matmul(
    a: Matrix[M, K, Real],
    b: Matrix[K, N, Real]
) -> c: Matrix[M, N, Real] {

    ($a, $b) -> grid<M, N> {
        cell(i, j) {
            a.row<i> -> $ar
            b.col<j> -> $bc
            ($ar, $bc) -> dot -> $out
        }
    } -> $c
}
```

`grid<M, N>` creates `M × N` independent cell computations.

Each cell computation may itself contain parallelism through `dot`.

---

# 16. Forbidden syntax

FlowArrow deliberately does not include:

```text
for
while
break
continue
return
throw
try/catch
await
spawn
join
lock
mutex
atomic
var
set
+=
i++
global mutable state
function pointers with unknown behavior
dynamic dispatch without purity/cost contracts
```

These constructs either create hidden ordering, hidden effects, or dynamic dependencies.

---

# 17. Allowed syntax summary

```flow
# imports
import std.bytes { split_lines, concat_bytes }
import std.cli { Args }
import std.io { read_stdin, write_stdout }
import std.math as math
import "./filters.flow" { blur, sobel as detect_edges }
import "./format.flow" as format

# type aliases
type Pixel = (Real,Real)

# pipeline
$x -> f -> $y

# fan-out
$x -> { f -> $a, g -> $b }

# join
($a, $b) -> h -> $y

# named input ports
$x -> op.left
$y -> op.right
op.out -> $z

# map
$xs -> map f -> $ys

# fault-aware map
$xs -> fault map parse_real { ok -> $values, fault -> $faults }

# reduce
$xs -> reduce associative_op(identity: $e) -> $y

# scan
$xs -> scan associative_op(identity: $e) -> $ys

# fixed or runtime repeat
$x -> repeat<$n> step -> $y      # repeat count may be a literal or runtime Int

# pure selection
($predicate, $true_value, $false_value) -> select -> $y

# static alternatives with runtime-selected evaluation
$x -> match {
    pred($arg) -> when_true
    _          -> when_false
} -> $y

# dynamic-size sequences
$n -> range -> $idxs
(0, $n, 2) -> range_step -> $even_idxs
$xs -> filter pred -> $ys
$xs -> length -> $n
[$a, $b, $c] -> concat_bytes -> $out
```

---

# 18. Compilation model

Every program compiles to:

```text
G = (V, E)
```

Where:

- `V` are pure computation nodes.
- `E` are explicit data dependencies.
- An edge `a -> b` means `b` cannot start until `a` is available.
- If no path connects two nodes, they are parallel.

The compiler computes:

```text
ready(node) = all input edges have values
```

Then schedules all ready nodes concurrently.

The theoretical execution time with unlimited processors is:

```text
critical_path_length(G)
```

When the program uses dynamic-size combinators (`range`, `filter`,
runtime `grid` / `repeat` dimensions), this length is a **closed-form
expression in the input sizes** rather than a constant — but the
expression itself is statically derivable, because the graph topology
is static.

No valid implementation can do better, because every edge is a real data dependency.

Therefore, FlowArrow exposes exactly the maximum legal parallelism expressible by the program.

---

# 19. Minimal grammar sketch

```ebnf
program     ::= declaration*

declaration ::= import_decl
              | "type" IDENT "=" TYPE
              | "node" IDENT "(" ports? ")" "->" ports block
              | "program" IDENT "(" ports? ")" "->" ports block

import_decl ::= "import" (module_path | STRING)
                ( "as" IDENT | "{" import_item ("," import_item)* "}" )

ports       ::= port ("," port)*
port_or_list::= port | "(" ports ")"
port        ::= IDENT ":" type

block       ::= "{" edge* "}"

edge        ::= chain

chain       ::= endpoint "->" stage ("->" stage)*

stage       ::= node_ref
              | variable_ref
              | combinator

endpoint    ::= variable_ref
              | literal
              | tuple
              | fanout
              | sequence

variable_ref::= "$" IDENT
node_ref    ::= IDENT ("." IDENT)*

tuple       ::= "(" endpoint ("," endpoint)+ ")"
sequence    ::= "[" "]" | "[" endpoint ("," endpoint)* "]"

fanout      ::= "{" stage ("->" stage)* ("," stage ("->" stage)*)* "}"

combinator  ::= "map" IDENT
              | "fault" "map" IDENT "{" "ok" "->" variable_ref ","
                "fault" "->" variable_ref "}"
              | "reduce" IDENT "(" "identity" ":" endpoint ")"
              | "scan" IDENT "(" "identity" ":" endpoint ")"
              | "repeat" "<" (INT | variable_ref) ">" IDENT
              | "select"
              | "range" | "range_between" | "range_step"
              | "filter" IDENT
              | "length"
```

---

# 20. Core invariant

A FlowArrow program is valid only if:

```text
syntax = data dependency
```

There is no syntax for sequencing.

There is no syntax for mutation.

There is no syntax for hidden effects.

There is no syntax for unknown dynamic control flow.

Therefore every valid FlowArrow program can be compiled into a fully explicit parallel execution graph.
