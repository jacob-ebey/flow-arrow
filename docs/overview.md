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
    input1 -> opA -> a
    input2 -> opB -> b
    (a, b) -> opC -> output
}
```

A block is **not sequential**. Lines merely declare graph edges.

This:

```flow
x -> f -> a
x -> g -> b
(a, b) -> h -> y
```

means:

```text
        ┌─> f ─> a ─┐
x ──────┤           ├─> h ─> y
        └─> g ─> b ─┘
```

`f` and `g` must execute in parallel if resources exist.

---

# 2. Values are single-assignment

```flow
x -> square -> sx
```

binds `sx` exactly once.

Illegal:

```flow
x -> square -> y
z -> sqrt -> y       # illegal: y already defined
```

There is no reassignment.

---

# 3. Fan-out

```flow
x -> {
    normalize -> n,
    histogram -> h,
    edges     -> e
}
```

Equivalent to three independent outgoing edges from `x`.

No branch is ordered before another.

---

# 4. Join

```flow
(a, b, c) -> combine -> result
```

`combine` waits only for `a`, `b`, and `c`.

No other dependency exists.

---

# 5. Named ports

For multi-input operations, ports may be explicit:

```flow
image  -> convolve.image
kernel -> convolve.kernel
convolve.out -> blurred
```

Equivalent shorthand:

```flow
(image, kernel) -> convolve -> blurred
```

Named ports are useful when order would be unclear.

---

# 6. Example: hypotenuse

```flow
node hypot(x: Real, y: Real) -> r: Real {
    x -> square -> xx
    y -> square -> yy
    (xx, yy) -> add -> sqrt -> r
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
x -> neg -> nx
x -> is_positive -> p
(p, x, nx) -> select -> absx
```

`select` is a pure dataflow node:

```text
select(predicate, when_true, when_false)
```

Both candidate values are ordinary graph inputs. The compiler sees the full dependency graph.

---

# 8. Data-parallel collections

Collections are first-class, but only through parallel-safe combinators.

## `map`

```flow
xs -> map square -> ys
```

Means each element is independent:

```text
ys[i] = square(xs[i])
```

No element may observe or mutate another.

---

## `zip`

```flow
(xs, ys) -> zip -> pairs
pairs -> map multiply_pair -> products
```

---

## `reduce`

Reductions require an associative operation and identity.

```flow
node dot(xs: Vec[N, Real], ys: Vec[N, Real]) -> s: Real {
    (xs, ys) -> zip
             -> map multiply_pair
             -> reduce add(identity: 0)
             -> s
}
```

`reduce` is compiled as a balanced tree.

Illegal:

```flow
xs -> reduce subtract(identity: 0) -> y
```

because subtraction is not associative.

---

## `scan`

Prefix operations are allowed only for associative operators:

```flow
xs -> scan add(identity: 0) -> prefix_sums
```

The compiler emits a parallel prefix tree.

---

## Dynamic sizes

Collection lengths may be runtime values. The *shape* of the dataflow
graph stays static; only the *width* of parallel regions varies.

`range` produces a sequence of integers from a runtime length:

```flow
n -> range -> idxs                    # idxs : Seq[Int], length = n
idxs -> map compute_pixel -> pixels
pixels -> reduce add(identity: 0) -> total
```

`range_between` and `range_step` produce sequences from `(start, stop)`
and `(start, stop, step)` respectively.

`filter` produces a sequence whose length is data-dependent. It is
compiled as a parallel predicate evaluation followed by a parallel
prefix-sum compaction — topology stays static:

```flow
xs -> filter is_positive -> positives
```

`length` reports the runtime length of a sequence:

```flow
xs -> length -> n
```

`grid<...>` dimensions and `repeat<...>` counts may also be runtime
values (see §9 and §14).

### What stays forbidden

Runtime values may parameterise the **size** of a parallel region or
the **count** of iterations. They may **not** select which nodes exist
or how they are connected:

```flow
# illegal — operator is chosen at runtime
op_name -> lookup_op -> op
xs -> map op -> ys

# illegal — sequential element-to-element dependency
xs -> take_while is_positive -> prefix
```

The full forbidden list lives in `syntax.md` §8.

---

# 9. Bounded iteration only

FlowArrow has no unbounded `while`.

Allowed with a compile-time count (the body unrolls into a fixed
dataflow chain):

```flow
state -> repeat<10> step -> final
```

Allowed with a runtime count (the body is a single static graph; the
runtime executes it `n` times):

```flow
(state, n) -> repeat<n> step -> final
```

In both cases the *graph shape* is static. Only the *number of
unrollings* may vary at runtime.

Illegal:

```flow
while not_done {
    state -> step -> state
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
    n1 -> fib -> a
    n2 -> fib -> b
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
    (img, kernel) -> stencil2d radius<1> convolve_pixel -> out
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
(seed, index) -> random_uniform -> value
```

Time must be explicit:

```flow
timestamp -> compute_deadline -> deadline
```

---

# 12. Effects are boundary-only

A complete program may have external inputs and outputs, but effects are outside the dataflow graph.

```flow
program main(input: Bytes) -> output: Bytes {
    input -> parse -> transform -> encode -> output
}
```

The runtime may connect `input` and `output` to files, sockets, or devices, but the FlowArrow graph itself is pure.

---

# 13. Image-processing example

```flow
node detect_edges(img: Image[H, W, RGB]) -> edges: Image[H, W, Gray] {
    img -> {
        grayscale -> gray,
        histogram -> hist
    }

    (gray, hist) -> equalize -> eq
    eq -> gaussian_blur radius<2> -> smooth
    smooth -> sobel -> edges
}
```

The compiler sees:

```text
             ┌─> grayscale ─> gray ─┐
img ─────────┤                       ├─> equalize ─> gaussian_blur ─> sobel ─> edges
             └─> histogram ─> hist ──┘
```

`grayscale` and `histogram` are independent.

---

# 14. Matrix multiplication example

```flow
node matmul(
    a: Matrix[M, K, Real],
    b: Matrix[K, N, Real]
) -> c: Matrix[M, N, Real] {

    (a, b) -> grid<M, N> {
        cell(i, j) {
            a.row<i> -> ar
            b.col<j> -> bc
            (ar, bc) -> dot -> out
        }
    } -> c
}
```

`grid<M, N>` creates `M × N` independent cell computations.

Each cell computation may itself contain parallelism through `dot`.

---

# 15. Forbidden syntax

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

# 16. Allowed syntax summary

```flow
# pipeline
x -> f -> y

# fan-out
x -> { f -> a, g -> b }

# join
(a, b) -> h -> y

# named input ports
x -> op.left
y -> op.right
op.out -> z

# map
xs -> map f -> ys

# reduce
xs -> reduce associative_op(identity: e) -> y

# scan
xs -> scan associative_op(identity: e) -> ys

# fixed or runtime repeat
x -> repeat<N> step -> y         # N may be a literal or a runtime Int

# pure selection
(predicate, true_value, false_value) -> select -> y

# dynamic-size sequences
n -> range -> idxs
xs -> filter pred -> ys
xs -> length -> n
```

---

# 17. Compilation model

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

# 18. Minimal grammar sketch

```ebnf
program     ::= declaration*

declaration ::= "node" IDENT "(" ports? ")" "->" ports block

ports       ::= port ("," port)*
port        ::= IDENT ":" TYPE

block       ::= "{" edge* "}"

edge        ::= chain

chain       ::= endpoint "->" endpoint ("->" endpoint)*

endpoint    ::= IDENT
              | IDENT "." IDENT
              | literal
              | tuple
              | fanout
              | combinator

tuple       ::= "(" endpoint ("," endpoint)+ ")"

fanout      ::= "{" chain ("," chain)* "}"

combinator  ::= "map" IDENT
              | "reduce" IDENT "(" "identity:" literal ")"
              | "scan" IDENT "(" "identity:" literal ")"
              | "repeat" "<" INT ">" IDENT
```

---

# 19. Core invariant

A FlowArrow program is valid only if:

```text
syntax = data dependency
```

There is no syntax for sequencing.

There is no syntax for mutation.

There is no syntax for hidden effects.

There is no syntax for unknown dynamic control flow.

Therefore every valid FlowArrow program can be compiled into a fully explicit parallel execution graph.
