//! Lets a host replace one field's rendered control with its own Yew
//! component — a custom widget, its own CSS, its own extra validation —
//! without forking `render_control`. Selected by the field's `sh:path` (an
//! exact predicate) or its `sh:datatype` (every field of that type), with
//! `Path` taking precedence when both could match: a host can set one
//! broad "every date uses my date-picker" rule and still special-case one
//! specific field by name.
//!
//! What this crate still renders around the override — the label,
//! required-marker, description, `unsupported` note, and (for a repeatable
//! field) the add/remove controls — is unchanged; only what
//! [`render_control`](crate::controls::render_control) would have returned
//! is replaced. See the workspace README's "Custom field controls"
//! section for a worked example.
use shacl_form_core::oxrdf::NamedNode;
use shacl_form_core::{Field, FieldKind, FormSchema, FormValues, ValueEntry, check_constraints};
use std::collections::HashMap;
use std::rc::Rc;
use yew::prelude::*;

/// Which field(s) a [`FieldOverride`] applies to.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum FieldSelector {
    /// This exact `sh:path` predicate, wherever it occurs in the schema —
    /// at the root or inside any `Nested` sub-form.
    Path(NamedNode),
    /// Every field whose `sh:datatype` is exactly this one, when no
    /// [`FieldSelector::Path`] entry also matches the same field.
    Datatype(NamedNode),
}

/// Builds the replacement control. Receives the same
/// (`Field`, current value, `onchange`) `render_control` itself receives,
/// so a custom component can still read the field's own derived
/// constraints (`patterns`, `min`/`max`, ...) if it wants to reflect them
/// (e.g. in its own placeholder text), even though this crate no longer
/// applies them as HTML attributes for this field.
pub type RenderOverride = Rc<dyn Fn(&Field, Option<&ValueEntry>, Callback<ValueEntry>) -> Html>;

/// An extra check run alongside [`shacl_form_core::check_constraints`] —
/// see [`FieldOverride::validate`].
pub type ValidateOverride = Rc<dyn Fn(&ValueEntry) -> Result<(), String>>;

/// A host-supplied replacement for one field's control.
#[derive(Clone)]
pub struct FieldOverride {
    pub render: RenderOverride,
    /// Runs in *addition* to [`shacl_form_core::check_constraints`] (this
    /// field's own `sh:pattern`/length/numeric bounds), not instead of it —
    /// both must pass. `None` when the override has nothing to add beyond
    /// the shape's own constraints.
    pub validate: Option<ValidateOverride>,
}

// `Properties` (on `ShaclFormProps`) needs `PartialEq` to decide when a
// re-render is skippable. Closures have no natural notion of equality, so
// this compares by `Rc` pointer identity — the same trick `yew::Callback`
// itself already uses internally, for the same reason: two `FieldOverride`s
// built from the same `Rc::new(...)` (the ordinary case: constructed once
// by the host and reused across renders) compare equal; two independently
// constructed ones, even with identical behaviour, do not.
impl PartialEq for FieldOverride {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.render, &other.render)
            && match (&self.validate, &other.validate) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

pub type FieldOverrides = HashMap<FieldSelector, FieldOverride>;

/// The override for `field`, if any — `Path` checked before `Datatype`,
/// per this module's own doc comment.
pub fn lookup<'a>(overrides: &'a FieldOverrides, field: &Field) -> Option<&'a FieldOverride> {
    field
        .path
        .as_ref()
        .and_then(|p| overrides.get(&FieldSelector::Path(p.clone())))
        .or_else(|| {
            field
                .original_datatype
                .as_ref()
                .and_then(|d| overrides.get(&FieldSelector::Datatype(d.clone())))
        })
}

/// Submit-time validation for every *overridden* field in the tree
/// (built-in fields keep relying on native HTML5 constraint validation,
/// unchanged) — `shacl_form_core::check_constraints` (this field's own
/// `sh:pattern`/length/numeric bounds) first, then the override's own
/// `validate`, if it has one. Recurses into every `Nested` field's own
/// values regardless of whether that field itself is overridden, since an
/// override can live deeper in the tree than the nesting it's inside.
/// Returns one `"{label}: {reason}"` string per failed entry, empty when
/// every overridden field's current value is valid.
pub fn validate_overridden(
    schema: &FormSchema,
    values: &FormValues,
    overrides: &FieldOverrides,
) -> Vec<String> {
    let mut errors = Vec::new();
    collect_errors(schema, values, overrides, &mut errors);
    errors
}

fn collect_errors(
    schema: &FormSchema,
    values: &FormValues,
    overrides: &FieldOverrides,
    errors: &mut Vec<String>,
) {
    for (idx, field) in schema.fields.iter().enumerate() {
        if let Some(over) = lookup(overrides, field) {
            for entry in values.get(idx) {
                if let Err(reason) = check_constraints(field, entry) {
                    errors.push(format!("{}: {reason}", field.label));
                } else if let Some(validate) = &over.validate
                    && let Err(reason) = validate(entry)
                {
                    errors.push(format!("{}: {reason}", field.label));
                }
            }
        }
        if let FieldKind::Nested {
            schema: nested_schema,
        } = &field.kind
        {
            for entry in values.get(idx) {
                if let ValueEntry::Nested {
                    values: nested_values,
                    ..
                } = entry
                {
                    collect_errors(nested_schema, nested_values, overrides, errors);
                }
            }
        }
    }
}
