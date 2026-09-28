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
| `sh:datatype` | the matching `Text`/`Number`/`Boolean`/`Date`/`DateTime`/`Iri` control — `xsd:anyURI` is `Text` (with that datatype preserved), not `Iri`: `sh:datatype` only ever constrains a *literal*, and `Iri` is a resource reference with no datatype at all |
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
| an instance subject reachable through its own `sh:node`/`sh:class` nesting again — a self-loop, or a longer cycle (`ex:alice ex:knows ex:alice`, or `A -> B -> A`) | not re-expanded (a real cycle, detected by ancestry on the current read path — its own identity is still kept, just no further fields), and a subject reached again at the same nesting depth via a *different*, non-ancestor path reuses the one already-read `Rc<FormValues>` instead of re-walking it — see `FormValues::read_from_instance`'s own doc comment |
| `sh:in`/`sh:hasValue` stated alongside `sh:class`/`sh:node` on one property | the nesting still wins (an exact shape is more useful than a fixed list), but the dropped enumeration is named in the field's `unsupported` note, not silently discarded |
| `sh:pattern`/`sh:hasValue` stated more than once on **one** property shape (legal RDF, unusual SHACL) | every `sh:pattern` value accumulates (the same as two different `sh:and` branches each stating one already did); more than one `sh:hasValue` is noted rather than one being silently kept |
| `sh:pattern`/`sh:minLength`/`sh:maxLength` on an `Iri` field (`sh:nodeKind sh:IRI`, or a `sh:class` with no matching shape) | applied as the `<input type="url">`'s own `pattern`/`minlength`/`maxlength`, the same as a `Text` field's; on any other kind, where there is genuinely no control to apply them to, named in `unsupported` instead of silently dropped |
| a numeric bound (`sh:minInclusive` and friends) whose literal isn't itself numeric (paired with `xsd:date`/`xsd:dateTime`, say — a legal pairing this crate's `Date`/`DateTime` controls don't apply a bound to yet) | named in `unsupported` as "stated but not a number this crate can compare against", distinctly from a bound that was never stated at all |
| a `sh:deactivated` node shape's own `sh:node`/`sh:and` ("shape inheritance") references | not expanded — a deactivated shape contributes nothing at all, not just its own direct `sh:property` list |
| `sh:datatype rdf:langString` | `Text`, recognised (not reported as an unrecognised datatype); editing a value read with a language tag keeps that tag (`literal_entry_with_language`) rather than silently discarding it for a plain typed literal, and a fresh value with no tag to keep serialises as a plain untyped literal rather than the illegal `"…"^^rdf:langString` |
| `Select` (`sh:in`/`sh:hasValue`) matching the *current* value to an option, and turning a picked option back into a value | by the option's own index, not its lexical string — two terms that stringify the same (`"chat"@fr` vs `"chat"@en`, or `1` vs `"1"`) are told apart instead of colliding onto one |

Nothing here is silently approximated: a construct this crate can't
faithfully turn into a control is named in `Field::unsupported` /
`FormSchema::unsupported`, and `FormSchema::all_unsupported()` flattens the
whole tree for a host that just wants one list to show a reader (and does so
in time proportional to the tree's *distinct* schemas, not to how many
`Nested` fields happen to point at a shared one — see its own doc comment).

## Custom field controls

`ShaclFormProps::field_overrides` replaces specific fields' rendered
controls with a host-supplied Yew component — a date-picker, an
autocomplete widget, whatever a plain `<input>` can't do — with its own
CSS and its own extra validation, without forking `render_control` or this
crate at all.

```rust
use shacl_form_yew::overrides::{FieldOverride, FieldOverrides, FieldSelector};
use shacl_form_core::oxrdf::NamedNode;
use shacl_form_core::ValueEntry;
use std::collections::HashMap;
use std::rc::Rc;

let mut overrides: FieldOverrides = HashMap::new();
overrides.insert(
    // Selected by sh:path (an exact field) or sh:datatype (every field of
    // that type) -- Path wins when both could match the same field.
    FieldSelector::Path(NamedNode::new("http://example.org/email").unwrap()),
    FieldOverride {
        render: Rc::new(|_field, current, onchange| {
            // Build whatever Html you want here -- your own component,
            // your own classes, your own CSS. `onchange` feeds a new
            // ValueEntry back into the form exactly like a built-in
            // control's own onchange does.
            html! { <MyEmailInput value={..} onchange={onchange} /> }
        }),
        // Runs IN ADDITION to this field's own sh:pattern/length/bounds
        // (shacl_form_core::check_constraints), not instead of them.
        validate: Some(Rc::new(|entry| {
            /* your own extra check */ Ok(())
        })),
    },
);
// ... pass `Some(Rc::new(overrides))` as ShaclFormProps::field_overrides
```

What stays built-in around the override: the field's label,
required-marker, description, `unsupported` note, and (for a repeatable
field) its add/remove controls — only what `render_control` would have
returned for that one field is replaced.

Native HTML5 constraint validation only ever applies to a real
`<input>`/`<select>` this crate itself rendered, so it can't see a
host-supplied control at all. Submitting the form therefore runs
`check_constraints` (and any `validate` an override supplies) against
every *overridden* field's current value first; a failure blocks
submission and lists the reasons (`.shacl-form-error`) instead of calling
`onsubmit`. Built-in fields are unaffected either way — still pure native
HTML5 validation, exactly as before this existed.

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
- `read_from_instance` shares one `Rc<FormValues>` across every occurrence
  of the same subject at the same nesting depth (see the coverage table),
  but the tree it builds is still a tree, not a graph: editing one
  occurrence clones only *that* occurrence's own spine (`paths::update_at`),
  so a sibling occurrence reached through a different, non-ancestor path to
  the identical subject keeps showing the pre-edit value until the form is
  reloaded. A densely cross-referenced instance (many subjects that mutually
  reference each other, read near `MAX_NESTING_DEPTH`) is no longer
  unbounded or slow to read — real cycles are caught and shared reads are
  reused, not re-walked — but can still render a genuinely large number of
  fields, one per distinct non-cyclic path through the data, not per
  distinct subject: closing both fully would need the renderer to
  understand "this is the same resource shown twice", which is a bigger,
  separate redesign.
- A blank-node-valued property this crate cannot nest (`sh:class` with no
  matching shape, `sh:nodeKind sh:BlankNodeOrIRI`, or a `sh:node`/`sh:class`
  reference past `MAX_NESTING_DEPTH`) is invisible to `read_from_instance`:
  the value is neither shown nor kept, so re-saving an otherwise-untouched
  instance silently drops it rather than round-tripping it unedited.
- `xsd:dateTimeStamp` (a timezone is mandatory) uses the same
  `datetime-local` control as `xsd:dateTime` (no timezone at all), so an
  edited value is never a legal lexical form for it; reading back an
  existing `xsd:dateTime`/`xsd:date` value that *does* carry a timezone or
  offset and re-typing even one character of it drops that offset, since
  `datetime-local`/`date` have nowhere to hold one.
- An integer field's `sh:minInclusive`/`sh:maxExclusive` etc. is used as
  HTML's `min`/`max` as stated, even when it is not itself a whole number
  (`sh:minExclusive 0.5` on an integer becomes `min=1.5`); combined with
  `step=1`, HTML then rejects every legal integer value. The number control
  also round-trips exponent notation (`1e5`, which `<input type=number>`
  accepts and returns as-is) as that literal string, which is not a valid
  `xsd:integer`/`xsd:decimal` lexical form.
- A current or default `Select` value that isn't among the field's own
  options at all (an instance written by something else, or a
  `sh:defaultValue` outside `sh:in`) still displays as unselected ("—")
  while being submitted unchanged if the field is never touched — matching
  by the option's own index (see the coverage table) tells two *listed*
  options apart but doesn't invent one for a value that isn't listed.

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
- **`read_from_instance` makes the same trade on the *data* side that
  `ShapeCache` makes on the *shape* side, plus one `ShapeCache` doesn't need:
  an instance graph can point back at itself.** Every `(nested schema,
  subject)` pair already fully read is cached and reused instead of
  re-walked (`values.rs`'s `ReadCache`), and a subject already being read
  higher up the *current* path is a real cycle, not a value to expand again
  — it's given an empty, identity-only value instead of recursing until
  `MAX_NESTING_DEPTH` cut it off, which used to read the *same* subject's
  fields once per depth level, independently, so editing only the
  outermost copy left every inner copy's stale value still asserted
  alongside it on save. See `shacl-form-core/tests/opus3_audit.rs`'s
  `a_self_referencing_instance_subject_does_not_duplicate_its_own_fields`
  and `mutually_referencing_instance_subjects_do_not_expand_exponentially`
  (the latter: 1,098,056 entries for eight mutually-cross-referencing
  subjects without this, well under a fifth of that with it).
- **A freshly created `Nested` value is given a real, random blank-node
  identity the moment it exists** (`default_entry`, via
  `oxrdf::BlankNode::default()`), not left identity-less until
  serialisation derives one from its position in the tree. A
  position-derived id can collide with an unrelated, already-real subject a
  *previous* save happened to mint at that same position (remove an entry,
  add a new one, and the new one lands where an old, still-referenced
  identity used to be) — two distinct resources silently merged under one
  label. See `opus3_audit.rs`'s
  `a_freshly_added_nested_entry_gets_its_own_identity_immediately_not_derived_from_position`.

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
