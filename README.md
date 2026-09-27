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
| `sh:node` / `sh:class` (resolved via every matching `sh:targetClass` shape, merged) | `Nested`, recursively — depth-bounded (see `MAX_NESTING_DEPTH`'s own doc comment) and cached by `(shape set, depth)` so a shape nested inside itself expands to one schema per depth level, shared, not a re-walked copy per occurrence |
| `sh:node` / `sh:and` **on the node shape itself** ("shape inheritance": `ex:EmployeeShape sh:node ex:PersonShape`) | the referenced shape's own `sh:property` list is pulled in and merged, the same as a sibling shape sharing a `sh:targetClass` |
| two `sh:property` values sharing one `sh:path` — in one shape, or across a merged `sh:targetClass`/inheritance set | merged into **one** field (tightening cardinality and constraints), not two competing fields writing duplicate triples |
| `sh:and` | merges every branch's constraints onto one field, **tightening** (the smallest `maxLength`/`maxInclusive`, the largest `minLength`/`minInclusive`, every `sh:pattern` kept, not just the last) rather than the last branch silently overwriting an earlier, stricter one |
| `sh:or` / `sh:xone` | renders the **first** branch only; the rest are named in the field's `unsupported` note |
| `sh:not`, `sh:sparql`, `sh:disjoint`, `sh:equals`, `sh:lessThan`, `sh:qualifiedValueShape`, `sh:flags`, any other `sh:`-namespaced predicate this crate doesn't branch on | reported, on the field or the shape, never silently dropped |
| `sh:message`, `sh:severity`, `sh:group`, `sh:deactivated` | not constraints — never reported as "unsupported"; a `sh:deactivated true` property is skipped entirely, per spec |
| `sh:minCount`/`sh:maxCount` | repeatable add/remove controls; a required field starts pre-filled, not an empty list |
| `sh:minLength`/`maxLength`/`pattern`/`minInclusive`/`maxInclusive`/`minExclusive`/`maxExclusive` | HTML `minlength`/`maxlength`/`min`/`max`, and `pattern` rebuilt as an unanchored-contains match (SHACL's own semantics — see `controls.rs`'s `combined_pattern`), not HTML's anchored one |
| `sh:defaultValue` | pre-fills a new instance (never overwrites an edited one) |
| `sh:order` | sorts fields; absent order sorts last, tie-broken by predicate for determinism (`oxrdf::Graph` iterates in an interned, not insertion, order — see `schema.rs`'s own `order_key`/`tie_break_key` docs) |
| more than one `sh:NodeShape` sharing a `sh:targetClass` | merged into one form (`from_target_class`), regardless of which shape happens to sort first, and regardless of whether every one of them states an explicit `a sh:NodeShape` (a shape asserting `sh:targetClass` is recognised structurally, per SHACL) |
| `rdf:type` for a `from_target_class` form's own subject, a `sh:class`-nested value, and a `sh:node`-nested value whose *referenced* shape states its own `sh:targetClass` | asserted on submit (`FormSchema::target_class`) — otherwise the produced instance would not conform to the very constraint that shaped it |
| `sh:node` and `sh:class` both stated on one property | nests via `sh:node`'s own shape (an exact reference beats "however many shapes target this class"), but still asserts `sh:class`'s class as `rdf:type` — noted, not silently dropped |
| re-saving an untouched value nested under `sh:node`/`sh:class` (edit mode) | reuses the subject it was read from (`ValueEntry::Nested`'s own `subject`); does not replace it with a fresh blank node, and mints the same stable id on repeated `to_turtle` calls for a value that has none yet (a resubmit after a failed save doesn't invent a second resource for the same unsaved value) |
| `sh:or ()` / `sh:xone ()` (an empty branch list) | reported as unsupported, not silently ignored |

Nothing here is silently approximated: a construct this crate can't
faithfully turn into a control is named in `Field::unsupported` /
`FormSchema::unsupported`, and `FormSchema::all_unsupported()` flattens the
whole tree for a host that just wants one list to show a reader (and does so
in time proportional to the tree's *distinct* schemas, not to how many
`Nested` fields happen to point at a shared one — see its own doc comment).

## Known limitations

- `sh:or`/`sh:xone` render only their first branch.
- `sh:flags` (e.g. case-insensitive matching) is reported but not applied to
  the rendered pattern.
- An exclusive numeric bound (`sh:minExclusive`/`sh:maxExclusive`) becomes an
  exact inclusive HTML bound for an **integer** field (`minExclusive 0` →
  `min=1`); for a decimal/float field there is no single "next representable
  value" to bump by, so it is approximated as the same inclusive bound —
  tighter than nothing, not exact.
- Serialised numeric/date/boolean literals use the field's own
  `sh:datatype` when the shape stated one, otherwise a canonical datatype
  per `FieldKind` (e.g. plain `xsd:integer` for an untyped numeric bound) —
  not a full type-inference pass.
- The `Iri` control accepts free text as you type (a half-typed IRI
  shouldn't fight you mid-keystroke); an invalid one is silently skipped at
  *serialisation* time rather than written as unparseable Turtle.
- No enforcement of `sh:closed`/`sh:disjoint`/`sh:equals`/`sh:lessThan`/
  `sh:sparql`/`sh:qualifiedValueShape` at all (see the coverage table).
- Repeated entries of one field are addressed positionally (a repetition
  *index*, captured when that entry's control was rendered), not by a
  stable per-entry id. `FormState::reduce` applies dispatched actions to
  whatever is actually current (see "Design notes" below) — the *lost
  update* case is fixed — but if a `Remove` and a second action captured
  from the *same* pre-removal render land in one dispatch batch (no
  re-render in between), the second action's index can point past where
  its intended entry now sits. `set_leaf`/`remove_entry` treat an
  out-of-range index as a no-op rather than fabricating a new entry (the
  bug an earlier audit pass found and this one fixed), but a no-op still
  means that second edit doesn't land — a stable-id redesign of repeated
  entries would close this properly; not done here.
- No `demo-ssg` (a host-only static pre-render, the way
  `ds-honeycomb-editor-rs` has one) yet.
- `shacl-form-yew` ships no CSS at all — the host supplies class names
  (`shacl-form-field`, `shacl-form-nested`, `shacl-form-entry`,
  `shacl-form-error`, `shacl-form-unsupported`, `shacl-form-required`,
  `shacl-form-add`/`shacl-form-remove`) the same way `ds-honeycomb-editor-rs`
  leaves colour and theme to its host.

## Design notes (why it's built this way)

- **State is one `Reducible` (`FormState`/`FormAction`, in `paths.rs`), not
  a directly-captured `FormValues` snapshot.** Yew defers a state update to
  a microtask; two edits dispatched synchronously (a script filling several
  fields, a test driver) would, against a captured snapshot, each build on
  the *pre-edit* state and the second `set()` would silently overwrite the
  first. A reducer applies each dispatched action to whatever is actually
  current when it is processed — see `shacl-form-yew/tests/dom.rs`'s
  `two_edits_dispatched_before_any_rerender_both_survive` for the case this
  fixes, reproduced and pinned in a real browser.
- **Every field's value row is `Rc<[ValueEntry]>`, and a `Nested` value
  carries its own `Rc<FormValues>`** (`shacl_form_core::FormValues`), so
  rebuilding the tree for one edit three levels deep clones the *spine*
  down to that field — a handful of pointer bumps per level — and shares
  every other field's row and every other nested repetition unchanged,
  rather than deep-copying the whole form on every keystroke.
- **Every expansion of a `sh:node`/`sh:class` reference is cached by
  `(expanded shape set, target class, nesting depth)`** in `schema.rs`'s
  `ShapeCache`. Without it, a shape with a handful of self-referencing
  properties (`foaf:knows` pointing back at `foaf:Person`, say) expands to
  roughly (branching factor)^`MAX_NESTING_DEPTH` real allocations — tens of
  thousands for a shape with four such properties — because every
  occurrence re-walked and re-allocated an identical subtree. With the
  cache, the same shape reached the same way at the same depth is computed
  once and shared; see `shacl-form-core/tests/robustness.rs`'s own
  `a_self_referencing_shape_with_several_such_properties_does_not_expand_exponentially`.

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
