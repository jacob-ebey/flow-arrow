# FlowArrow — Formal Syntax Specification

This document formally defines the concrete syntax of **FlowArrow**.
It is the normative reference for parsers and tooling; the prose
discussion of semantics lives in [`overview.md`](./overview.md).

The grammar is presented in **EBNF** with the following conventions:

| Notation        | Meaning                                |
| --------------- | -------------------------------------- |
| `"x"`           | literal terminal                       |
| `A B`           | concatenation                          |
| `A \| B`        | alternation                            |
| `A?`            | zero or one                            |
| `A*`            | zero or more                           |
| `A+`            | one or more                            |
| `( ... )`       | grouping                               |
| `UPPER`         | lexical (terminal) production          |
| `lower`         | syntactic (non-terminal) production    |

A FlowArrow source file is a sequence of Unicode code points encoded
as UTF-8.

---

## 1. Lexical structure

### 1.1 Whitespace and comments

Whitespace and comments are insignificant except as token separators.

```ebnf
WS           ::= (" " | "\t" | "\r" | "\n")+
LINE_COMMENT ::= "#" (any character except "\n")* "\n"
BLOCK_COMMENT::= "/*" (any character)*? "*/"
```

Line terminators do **not** terminate statements: a `chain` is delimited
by its arrow structure, not by newlines. Newlines are therefore
equivalent to spaces.

### 1.2 Identifiers

```ebnf
IDENT      ::= ID_START ID_CONTINUE*
ID_START   ::= letter | "_"
ID_CONTINUE::= letter | digit | "_"
letter     ::= "A".."Z" | "a".."z"
digit      ::= "0".."9"
```

Identifiers are case-sensitive. The following are **reserved keywords**
and may not be used as identifiers:

```text
node      program   map       reduce    scan      repeat
select    identity  grid      cell      stencil2d
range     range_between        range_step          filter
length
```

### 1.3 Literals

```ebnf
literal    ::= INT | REAL | BOOL | STRING
INT        ::= "-"? digit+
REAL       ::= "-"? digit+ "." digit+ (("e" | "E") "-"? digit+)?
BOOL       ::= "true" | "false"
STRING     ::= "\"" (any character except "\"" or "\\" | escape)* "\""
escape     ::= "\\" ("\"" | "\\" | "n" | "t" | "r")
```

### 1.4 Punctuation and operators

```text
->   .   ,   :   (   )   {   }   <   >
```

The only multi-character token is `->` (the **flow arrow**).

---

## 2. Top level

```ebnf
program        ::= declaration*

declaration    ::= node_decl
                 | program_decl

node_decl      ::= "node" IDENT "(" port_list? ")" "->" port_or_list block

program_decl   ::= "program" IDENT "(" port_list? ")" "->" port_or_list block
```

A `program_decl` has identical syntax to a `node_decl`; the difference
is semantic (it is the program entry point and its inputs/outputs are
the boundary across which effects may occur — see `overview.md` §12).

```ebnf
port_or_list   ::= port
                 | "(" port_list ")"

port_list      ::= port ("," port)*

port           ::= IDENT ":" type
```

---

## 3. Types

```ebnf
type           ::= IDENT type_args?

type_args      ::= "[" type_arg ("," type_arg)* "]"

type_arg       ::= type
                 | INT
                 | IDENT
```

Examples (informative):

```text
Real
Int
Bool
Image[H, W]
Image[H, W, RGB]
Vec[N, Real]
Matrix[M, K, Real]
Kernel[3, 3]
```

`H`, `W`, `N`, `M`, `K` are type-level identifiers (shape variables).
Numeric type arguments are non-negative integer literals.

---

## 4. Blocks and chains

A block is an unordered collection of dataflow edges expressed as chains.

```ebnf
block          ::= "{" chain* "}"

chain          ::= endpoint ("->" stage)+

stage          ::= endpoint
                 | combinator
```

Each `chain` declares a path through the dependency graph. Successive
`->` operators are left-associative: `a -> b -> c` is read as the two
edges `a -> b` and `b -> c`. Chains within a block may appear in any
order; ordering carries no semantic meaning.

A chain must terminate at an endpoint that **binds** a name (see §7).

---

## 5. Endpoints

```ebnf
endpoint       ::= name_ref
                 | tuple
                 | fanout
                 | seq_literal
                 | literal

name_ref       ::= IDENT ("." IDENT)?
```

- `IDENT` — a value name.
- `IDENT "." IDENT` — a **named port** on a node (`op.port`).
- `tuple` — a join of multiple values (§5.1).
- `fanout` — a fan-out from a single value (§5.2).
- `seq_literal` — a fixed-arity sequence value (§5.3).
- `literal` — a constant value used as an input.

A bare `IDENT` used as the **target** of a `->` denotes either an
existing node application or the name being bound (§7).

### 5.1 Tuples (joins)

```ebnf
tuple          ::= "(" endpoint "," endpoint ("," endpoint)* ")"
```

A tuple of arity ≥ 2 represents the multi-input edge into the next
stage. `(a, b) -> h -> y` means `h` waits for both `a` and `b`.

### 5.2 Fanouts

```ebnf
fanout         ::= "{" fanout_arm ("," fanout_arm)* "}"

fanout_arm     ::= stage ("->" stage)*
```

A fanout duplicates its upstream value into multiple parallel arms.
Each arm is itself a (sub)chain that must terminate in a binding.

```text
x -> { f -> a, g -> b }
```

declares two independent edges from `x`.

### 5.3 Sequence literals

```ebnf
seq_literal    ::= "[" "]"
                 | "[" endpoint ("," endpoint)* "]"
```

A sequence literal constructs a `Seq[T]` of fixed arity from its
element endpoints. All elements must have the same type `T`. The
arity is known at compile time, but the resulting value flows as an
ordinary `Seq[T]` and is indistinguishable from one produced by
`range`, `filter`, or `map`.

```flow
[a, b, c] -> concat_bytes -> out
```

is equivalent to constructing a length-3 sequence and passing it to
`concat_bytes : Seq[Bytes] -> Bytes`. The empty sequence is `[]`.

---

## 6. Combinators

Combinators are stage forms that the parser recognises specially.
They are not first-class identifiers and may appear only between
`->` arrows.

```ebnf
combinator     ::= map_comb
                 | reduce_comb
                 | scan_comb
                 | repeat_comb
                 | select_comb
                 | stencil_comb
                 | grid_comb
                 | range_comb
                 | filter_comb
                 | length_comb

map_comb       ::= "map" IDENT

reduce_comb    ::= "reduce" IDENT "(" "identity" ":" literal ")"

scan_comb      ::= "scan"   IDENT "(" "identity" ":" literal ")"

repeat_comb    ::= "repeat" "<" repeat_count ">" IDENT
repeat_count   ::= INT | IDENT

select_comb    ::= "select"

stencil_comb   ::= "stencil2d" "radius" "<" INT ">" IDENT

grid_comb      ::= "grid" "<" grid_dim ("," grid_dim)* ">" grid_body

grid_dim       ::= IDENT | INT

grid_body      ::= "{" cell_decl "}"

cell_decl      ::= "cell" "(" IDENT ("," IDENT)* ")" block

range_comb     ::= "range"
                 | "range_between"
                 | "range_step"

filter_comb    ::= "filter" IDENT

length_comb    ::= "length"
```

Notes:

- `reduce` and `scan` require their `IDENT` operator to be associative
  (a semantic check, not a syntactic one).
- `repeat<N>` accepts either an integer literal **or** a `name_ref` of
  type `Int`. When `N` is a runtime value, the iteration count varies
  per invocation but the body graph is still static.
- `select` is invoked as `(predicate, when_true, when_false) -> select`.
- `grid<...>` introduces shape-indexed parallelism. Each `grid_dim`
  may be an integer literal, a compile-time identifier (shape
  variable), or a runtime `Int` value. Topology is fixed; only the
  width of the parallel region varies.
- `range` takes a single `Int` and produces `Seq[Int]` of that length
  (`0..n`). `range_between` takes `(start, stop)`; `range_step` takes
  `(start, stop, step)`. Output length is data-dependent but graph
  topology is not.
- `filter pred` takes a `Seq[T]` and a pure predicate node `pred`,
  producing a `Seq[T]` whose length is data-dependent. Compiled as a
  parallel predicate evaluation followed by a parallel prefix-sum
  compaction.
- `length` takes a `Seq[T]` and produces its `Int` length.

### 6.0.1 Dynamic sizes vs. dynamic topology (normative)

FlowArrow distinguishes two kinds of "dynamic":

| Kind                                | Allowed? |
| ----------------------------------- | -------- |
| Runtime **values** flowing on edges | yes (this is just dataflow) |
| Runtime **sizes** of collections    | yes (`range`, `filter`, runtime `grid` / `repeat` dims) |
| Runtime **topology** of the graph   | **no**   |

Equivalently: a runtime value may parameterise the *width* of a
parallel region or the *count* of iterations, but it may never select
which nodes exist or how they are connected. The compiler must always
be able to write down the full DAG template before execution begins;
only the multiplicities of `map` / `grid` / `repeat` instances may be
deferred to runtime.

The following are therefore still **forbidden**:

- Dynamic dispatch (`op_name -> lookup_op -> op; xs -> map op -> ys`).
- Data-dependent branching that selects a subgraph (use `select` —
  both branches are always evaluated, no topology change).
- `take_while`, `find_first`, or any combinator with sequential
  element-to-element dependencies.

### 6.1 Indexed port references (informative)

Within a `grid` `cell`, port references may carry compile-time index
arguments using angle brackets:

```ebnf
indexed_port   ::= IDENT "." IDENT "<" grid_dim ">"
```

These are parsed where a `name_ref` is expected.

---

## 7. Binding rules

Within a single `node_decl` or `program_decl`:

1. Each `IDENT` appearing as the **rightmost endpoint** of a chain or
   fanout arm is a **binding occurrence**: it introduces a new name.
2. A name may be bound **exactly once** (single static assignment).
3. Every other occurrence of an `IDENT` is a **use**: it must resolve
   either to an input port of the enclosing node, or to a name bound
   elsewhere in the same block.
4. The set of bindings and uses forms the data-dependency graph; it
   must be acyclic. Cycles are syntactically expressible but
   semantically rejected.

These rules are enforced by static analysis after parsing.

---

## 8. Forbidden syntax

The following tokens and constructs are **not** part of FlowArrow and
must be rejected by the parser:

```text
for      while    break    continue   return
throw    try      catch    await      spawn
join     lock     mutex    atomic     var
set      =        +=       -=         *=        /=
++       --       if       else
take_while         find_first
```

A FlowArrow source containing any of these tokens (outside of comments
or string literals) is **ill-formed**.

The single permitted form of conditional selection is the pure
`select` combinator (§6).

---

## 9. Consolidated grammar

For convenience, the complete syntactic grammar is reproduced here.

```ebnf
program        ::= declaration*

declaration    ::= node_decl | program_decl

node_decl      ::= "node"    IDENT "(" port_list? ")" "->" port_or_list block
program_decl   ::= "program" IDENT "(" port_list? ")" "->" port_or_list block

port_or_list   ::= port | "(" port_list ")"
port_list      ::= port ("," port)*
port           ::= IDENT ":" type

type           ::= IDENT type_args?
type_args      ::= "[" type_arg ("," type_arg)* "]"
type_arg       ::= type | INT | IDENT

block          ::= "{" chain* "}"
chain          ::= endpoint ("->" stage)+
stage          ::= endpoint | combinator

endpoint       ::= name_ref | tuple | fanout | seq_literal | literal
name_ref       ::= IDENT ("." IDENT)?
tuple          ::= "(" endpoint "," endpoint ("," endpoint)* ")"
fanout         ::= "{" fanout_arm ("," fanout_arm)* "}"
fanout_arm     ::= stage ("->" stage)*
seq_literal    ::= "[" "]" | "[" endpoint ("," endpoint)* "]"

combinator     ::= map_comb | reduce_comb | scan_comb
                 | repeat_comb | select_comb
                 | stencil_comb | grid_comb
                 | range_comb | filter_comb | length_comb

map_comb       ::= "map" IDENT
reduce_comb    ::= "reduce" IDENT "(" "identity" ":" literal ")"
scan_comb      ::= "scan"   IDENT "(" "identity" ":" literal ")"
repeat_comb    ::= "repeat" "<" repeat_count ">" IDENT
repeat_count   ::= INT | IDENT
select_comb    ::= "select"
stencil_comb   ::= "stencil2d" "radius" "<" INT ">" IDENT
grid_comb      ::= "grid" "<" grid_dim ("," grid_dim)* ">" grid_body
grid_dim       ::= IDENT | INT
grid_body      ::= "{" cell_decl "}"
cell_decl      ::= "cell" "(" IDENT ("," IDENT)* ")" block
range_comb     ::= "range" | "range_between" | "range_step"
filter_comb    ::= "filter" IDENT
length_comb    ::= "length"
```

---

## 10. Core invariant

A FlowArrow source is well-formed if and only if:

1. It parses against the grammar in §9.
2. It contains no token from the forbidden set in §8.
3. Every name satisfies the single-assignment binding rules in §7.
4. The induced dependency graph is acyclic.

Under these conditions, and only under these conditions:

```text
syntax = data dependency
```
