# shacl-form-rs

A [Yew](https://yew.rs) component that turns an **arbitrary SHACL shapes
graph** into a real HTML form: give it Turtle and a `sh:NodeShape` (or a
class every matching shape targets), get back labelled, typed, validated
inputs, and — on submit — the RDF instance data the filled-in form
describes. Not the fixed, hand-drawn honeycomb: a form generated from
whatever shape you hand it.

**Demo: <https://ds-labs-org.github.io/shacl-form-rs/>**

## Layout

| path | what it is |
|---|---|
| `shacl-form-core/` | parses the shapes graph, builds a `FormSchema`, collects and serialises values. No UI framework |
| `shacl-form-yew/` | the component: renders a `FormSchema`, turns DOM events into values, recurses into nested shapes |
| `demo/` | the Trunk app: paste a shapes graph, pick a target shape, edit the live form, read the Turtle it produces |

## Why a real RDF crate

`ds-labs-org/ds-honeycomb-editor-rs` — this repository's sibling, one fixed
vocabulary it owns — hand-rolls its own Turtle reader and writer specifically
*to avoid* a real RDF dependency, and its own README says why: "the obvious
one reaches wasm32 through `getrandom` and does not compile there at all."
That's true of `getrandom`'s **default** backend, but not the whole story —
`getrandom`'s `"js"` feature switches it to `crypto.getRandomValues` via
`wasm-bindgen`, which is exactly what a browser tab already has, and a
`cfg(target_arch = "wasm32")`-gated dependency keeps that JS-glue requirement
off a host-side (non-browser) build entirely. Verified with a throwaway
`cargo check --target wasm32-unknown-unknown` spike against
[`oxrdf`](https://crates.io/crates/oxrdf) + [`oxttl`](https://crates.io/crates/oxttl)
(the term/graph model and Turtle reader/writer from the
[oxigraph](https://github.com/oxigraph/oxigraph) project, neither pulling in
its SPARQL engine or the actual triple store) before choosing them over a
hand-rolled parser.

That choice is also the right one for a *different reason* than compiling at
all: `ds-honeycomb-editor-rs` only ever reads and writes its own fixed
vocabulary, so a parser scoped to exactly those terms is a small, safe
surface. This crate's whole point is reading a shapes graph it did not write
and was not designed around — an arbitrary SHACL Core subset, arbitrary
prefixes, arbitrary nesting. Hand-rolling a Turtle parser robust enough for
that is a materially bigger, riskier undertaking than reusing a mature one;
`oxrdf`/`oxttl` earn their weight here in a way they would not for a
single-vocabulary tool.

## SHACL coverage

Reads real SHACL Core, not a toy subset — verified against
`ds-honeycomb-editor-rs`'s own `shapes.ttl` (783 lines, `sh:closed`,
`sh:xone`, four `sh:sparql`-only node shapes sharing one `sh:targetClass`;
see `shacl-form-core/tests/honeycomb.rs`), not just hand-written fixtures.

| construct | handled as |
|---|---|
| `sh:path <iri>` | the field's predicate |
| `sh:path` (blank-node: sequence/alternative/inverse/`*`/`+`/`?`) | field still listed, reported unsupported, not editable |
| `sh:datatype` | the matching `Text`/`Number`/`Boolean`/`Date`/`DateTime`/`Iri` control |
| `sh:nodeKind sh:IRI` | `Iri`; `BlankNode`/mixed kinds reported unsupported, rendered as text |
| `sh:in` | `Select`, options exactly as listed |
| `sh:hasValue` | `Select` with that one option |
| `sh:node` / `sh:class` (resolved via a matching `sh:targetClass`) | `Nested`, recursively — depth-bounded (see `MAX_NESTING_DEPTH`'s own doc comment), so a shape nested inside itself still terminates instead of refusing to render at all |
| `sh:and` | merges every branch's constraints onto one field |
| `sh:or` / `sh:xone` | renders the **first** branch only; the rest are named in the field's `unsupported` note |
| `sh:not`, `sh:sparql`, `sh:disjoint`, `sh:equals`, `sh:lessThan`, `sh:qualifiedValueShape`, any other `sh:`-namespaced predicate this crate doesn't branch on | reported, on the field or the shape, never silently dropped |
| `sh:minCount`/`sh:maxCount` | repeatable add/remove controls; a required field starts pre-filled, not an empty list |
| `sh:minLength`/`maxLength`/`pattern`/`minInclusive`/`maxInclusive`/`minExclusive`/`maxExclusive` | HTML `pattern`/`minlength`/`maxlength`/`min`/`max` |
| `sh:defaultValue` | pre-fills a new instance (never overwrites an edited one) |
| `sh:order` | sorts fields; absent order sorts last, tie-broken by predicate for determinism (`oxrdf::Graph` iterates in an interned, not insertion, order — see `schema.rs`'s own `order_key`/`tie_break_key` docs) |
| more than one `sh:NodeShape` sharing a `sh:targetClass` | merged into one form (`from_target_class`) — a real shape in `ds-honeycomb-editor-rs`'s own `shapes.ttl` |

Nothing here is silently approximated: a construct this crate can't
faithfully turn into a control is named in `Field::unsupported` /
`FormSchema::unsupported`, and `FormSchema::all_unsupported()` flattens the
whole tree for a host that just wants one list to show a reader.

## Known limitations

- `sh:or`/`sh:xone` render only their first branch.
- Serialised numeric/date/boolean literals use the field's own
  `sh:datatype` when the shape stated one, otherwise a canonical datatype
  per `FieldKind` (e.g. plain `xsd:integer` for an untyped numeric bound) —
  not a full type-inference pass.
- No client-side IRI validation on the `Iri` control (accepts any string);
  no enforcement of `sh:closed`/`sh:disjoint`/`sh:equals`/`sh:lessThan`/
  `sh:sparql` at all (see the coverage table).
- No `demo-ssg` (a host-only static pre-render, the way
  `ds-honeycomb-editor-rs` has one) yet.
- `shacl-form-yew` ships no CSS at all — the host supplies class names
  (`shacl-form-field`, `shacl-form-nested`, `shacl-form-entry`,
  `shacl-form-error`, `shacl-form-unsupported`, `shacl-form-required`,
  `shacl-form-add`/`shacl-form-remove`) the same way `ds-honeycomb-editor-rs`
  leaves colour and theme to its host.

## Requirements

- [Rust](https://www.rust-lang.org/) — [install](https://rustup.rs/)
- [Trunk](https://trunkrs.dev/) (for the demo): `cargo install trunk`
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`

## Develop

```bash
cd demo && trunk serve
```

## Test

```bash
cargo test --workspace --locked                     # shacl-form-core, host
CHROMEDRIVER=$(which chromedriver) \
  cargo test -p shacl-form-yew --target wasm32-unknown-unknown \
    --features csr --locked --test dom               # real browser, real DOM
```

## License

Apache-2.0 — see [LICENSE](LICENSE).
