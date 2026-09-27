//! One [`shacl_form_core::Field`]'s worth of leaf HTML — a single
//! `<input>`/`<select>`, not the label, the add/remove buttons, or the
//! recursion into a `Nested` field's own fields. All of that lives in
//! [`crate::render`], which calls into here once per rendered value.
use shacl_form_core::oxrdf::{Literal, NamedNode, Term};
use shacl_form_core::{Field, FieldKind, ValueEntry, literal_entry};
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
/// input becomes a value.
pub fn render_control(
    field: &Field,
    current: Option<&ValueEntry>,
    onchange: Callback<ValueEntry>,
) -> Html {
    match &field.kind {
        FieldKind::Text {
            pattern,
            min_length,
            max_length,
        } => {
            let field = field.clone();
            let oninput =
                onchange.reform(move |e: InputEvent| literal_entry(&field, &input_value(e)));
            html! {
                <input type="text" value={lexical(current)} oninput={oninput}
                    pattern={pattern.clone()}
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
        FieldKind::Iri => {
            let oninput = onchange
                .reform(|e: InputEvent| ValueEntry::Node(NamedNode::new_unchecked(input_value(e))));
            html! { <input type="url" value={lexical(current)} oninput={oninput} placeholder="https://…" /> }
        }
        FieldKind::Select { options } => {
            let current_str = lexical(current);
            let options = options.clone();
            let owned = options.clone();
            let onchange = onchange.reform(move |e: Event| {
                let chosen = select_value(e);
                owned
                    .iter()
                    .find(|o| term_as_string(&o.value) == chosen)
                    .map(|o| term_to_entry(&o.value))
                    .unwrap_or_else(|| ValueEntry::Literal(Literal::new_simple_literal(chosen)))
            });
            html! {
                <select onchange={onchange}>
                    <option value="" selected={current_str.is_empty()}>{ "—" }</option>
                    { for options.iter().map(|o| {
                        let v = term_as_string(&o.value);
                        html! { <option value={v.clone()} selected={v == current_str}>{ &o.label }</option> }
                    }) }
                </select>
            }
        }
        FieldKind::Nested { .. } => html! {}, // rendered by crate::render, never here
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
