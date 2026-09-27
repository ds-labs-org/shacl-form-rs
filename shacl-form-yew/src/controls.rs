//! One [`shacl_form_core::Field`]'s worth of leaf HTML — a single
//! `<input>`/`<select>`, not the label, the add/remove buttons, or the
//! recursion into a `Nested` field's own fields. All of that lives in
//! [`crate::render`], which calls into here once per rendered value.
use shacl_form_core::oxrdf::{Literal, NamedNode, Term};
use shacl_form_core::{Field, FieldKind, ValueEntry, literal_entry, literal_entry_with_language};
use web_sys::{Event, HtmlInputElement, HtmlSelectElement, MouseEvent};
use yew::prelude::*;

/// The lexical string a control should start showing — the same string a
/// SHACL literal's own `.value()` carries, or an IRI's own string form.
/// There is no meaningful lexical form for a `Nested` entry (it has no
/// single value at all); `render_control` is never called for one.
pub fn lexical(entry: Option<&ValueEntry>) -> String {
    match entry {
        Some(ValueEntry::Literal(l)) => l.value().to_string(),
        Some(ValueEntry::Node(n)) => n.as_str().to_string(),
        _ => String::new(),
    }
}

fn input_value(e: InputEvent) -> String {
    e.target_dyn_into::<HtmlInputElement>()
        .map(|el| el.value())
        .unwrap_or_default()
}

fn select_value(e: Event) -> String {
    e.target_dyn_into::<HtmlSelectElement>()
        .map(|el| el.value())
        .unwrap_or_default()
}

/// Renders the one control this field's `kind` calls for, and turns
/// whatever the browser hands back on input into the [`ValueEntry`] that
/// same kind expects — the two are written together deliberately, so a new
/// `FieldKind` variant cannot add a control without also saying how its
/// input becomes a value. `on_clear` fires only from `Select`'s own "—"
/// (no selection) option — see its own doc comment for why that can't just
/// be another `onchange` value.
pub fn render_control(
    field: &Field,
    current: Option<&ValueEntry>,
    onchange: Callback<ValueEntry>,
    on_clear: Callback<()>,
) -> Html {
    match &field.kind {
        FieldKind::Text {
            patterns,
            min_length,
            max_length,
        } => {
            let field = field.clone();
            // Captured now, not read from `current` inside the closure —
            // `current` only borrows for this render, but the closure must
            // outlive it. Kept so retyping a language-tagged value (e.g.
            // one read back from an existing instance) doesn't silently
            // drop its language tag on the very next keystroke.
            let language = match current {
                Some(ValueEntry::Literal(l)) => l.language().map(str::to_string),
                _ => None,
            };
            let oninput = onchange.reform(move |e: InputEvent| {
                literal_entry_with_language(&field, &input_value(e), language.as_deref())
            });
            html! {
                <input type="text" value={lexical(current)} oninput={oninput}
                    pattern={combined_pattern(patterns)}
                    minlength={min_length.map(|n| n.to_string())}
                    maxlength={max_length.map(|n| n.to_string())} />
            }
        }
        FieldKind::Number {
            integer_only,
            min,
            max,
        } => {
            let field = field.clone();
            let step = if *integer_only { "1" } else { "any" };
            let oninput =
                onchange.reform(move |e: InputEvent| literal_entry(&field, &input_value(e)));
            html! {
                <input type="number" value={lexical(current)} oninput={oninput}
                    step={step} min={min.map(|n| n.to_string())} max={max.map(|n| n.to_string())} />
            }
        }
        FieldKind::Boolean => {
            let field = field.clone();
            let checked = matches!(current, Some(ValueEntry::Literal(l)) if l.value() == "true" || l.value() == "1");
            let onclick = onchange.reform(move |e: MouseEvent| {
                let checked = e
                    .target_dyn_into::<HtmlInputElement>()
                    .map(|el| el.checked())
                    .unwrap_or_default();
                literal_entry(&field, if checked { "true" } else { "false" })
            });
            html! { <input type="checkbox" checked={checked} onclick={onclick} /> }
        }
        FieldKind::Date => {
            let field = field.clone();
            let oninput =
                onchange.reform(move |e: InputEvent| literal_entry(&field, &input_value(e)));
            html! { <input type="date" value={lexical(current)} oninput={oninput} /> }
        }
        FieldKind::DateTime => {
            let field = field.clone();
            let oninput =
                onchange.reform(move |e: InputEvent| literal_entry(&field, &input_value(e)));
            html! { <input type="datetime-local" value={lexical(current)} oninput={oninput} /> }
        }
        FieldKind::Iri {
            patterns,
            min_length,
            max_length,
        } => {
            let oninput = onchange
                .reform(|e: InputEvent| ValueEntry::Node(NamedNode::new_unchecked(input_value(e))));
            html! {
                <input type="url" value={lexical(current)} oninput={oninput} placeholder="https://…"
                    pattern={combined_pattern(patterns)}
                    minlength={min_length.map(|n| n.to_string())}
                    maxlength={max_length.map(|n| n.to_string())} />
            }
        }
        FieldKind::Select { options } => {
            // Matched by the option's own *index*, not its lexical string:
            // two `sh:in` terms that stringify the same (`1` and `"1"`, or
            // `"chat"@fr` and `"chat"@en`) are still different terms, and
            // used to collide onto whichever one `term_as_string` happened
            // to compare equal first — both looked selected, and only the
            // first was ever reachable at all. Comparing the actual `Term`
            // (datatype/language included) is what tells them apart.
            let current_term = entry_as_term(current);
            let selected_idx = options
                .iter()
                .position(|o| Some(&o.value) == current_term.as_ref());
            let owned = options.clone();
            // Picking "—" (no selection) must REMOVE this entry, not write
            // one holding an empty literal (`""`) — an empty string is a
            // real, if useless, value, and serialising it satisfies
            // `sh:minCount` for nothing. `select_value` returning "" is
            // unambiguous here: the blank option is the only one whose
            // `value` is ever the empty string (every real option's is now
            // its index, `"0"`, `"1"`, ...), so "the user picked the blank
            // option" and "onchange somehow got a stray empty string" are
            // the same case and both mean "clear".
            let onselect = Callback::from(move |e: Event| {
                let chosen = select_value(e);
                if chosen.is_empty() {
                    on_clear.emit(());
                } else if let Some(entry) = chosen
                    .parse::<usize>()
                    .ok()
                    .and_then(|idx| owned.get(idx))
                    .map(|o| term_to_entry(&o.value))
                {
                    onchange.emit(entry);
                }
            });
            html! {
                <select onchange={onselect}>
                    <option value="" selected={selected_idx.is_none()}>{ "—" }</option>
                    { for options.iter().enumerate().map(|(idx, o)| {
                        html! { <option value={idx.to_string()} selected={selected_idx == Some(idx)}>{ &o.label }</option> }
                    }) }
                </select>
            }
        }
        FieldKind::Nested { .. } => html! {}, // rendered by crate::render, never here
    }
}

/// SHACL's `sh:pattern` is an *unanchored* search (the value merely needs
/// to contain a match somewhere) using XPath/JS-flavoured regex; HTML's own
/// `pattern` attribute always matches the *whole* value (the browser wraps
/// it as `^(?:…)$`). Wrapping each pattern in a zero-width lookahead
/// (`(?=.*(?:p))`) and ending in a bare `.*` reproduces "contains a match
/// for every pattern, in any order, anywhere" under that anchored
/// wrapping — and composes multiple patterns (from `sh:and`) as a
/// conjunction instead of only being able to state one.
fn combined_pattern(patterns: &[String]) -> Option<String> {
    if patterns.is_empty() {
        return None;
    }
    let lookaheads: String = patterns.iter().map(|p| format!("(?=.*(?:{p}))")).collect();
    Some(format!("{lookaheads}.*"))
}

/// A `Select`'s current entry, as the `Term` it was built from — used to
/// find which option (if any) it matches, by real term equality
/// (datatype/language included), not by lexical string.
fn entry_as_term(entry: Option<&ValueEntry>) -> Option<Term> {
    match entry {
        Some(ValueEntry::Literal(l)) => Some(Term::Literal(l.clone())),
        Some(ValueEntry::Node(n)) => Some(Term::NamedNode(n.clone())),
        _ => None,
    }
}

fn term_as_string(term: &Term) -> String {
    match term {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => b.as_str().to_string(),
    }
}

fn term_to_entry(term: &Term) -> ValueEntry {
    match term {
        Term::NamedNode(n) => ValueEntry::Node(n.clone()),
        Term::Literal(l) => ValueEntry::Literal(l.clone()),
        Term::BlankNode(_) => {
            ValueEntry::Literal(Literal::new_simple_literal(term_as_string(term)))
        }
    }
}
