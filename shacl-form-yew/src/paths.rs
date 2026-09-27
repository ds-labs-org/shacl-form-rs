//! Every edit the rendered form makes is "replace the whole [`FormValues`]
//! tree with a new one that differs at exactly one leaf" — no interior
//! mutability, no index bookkeeping leaking into the component tree. A
//! [`Loc`] names that one leaf; [`update_at`] is the one function that
//! knows how to get there.
use shacl_form_core::{FieldKind, FormSchema, FormValues, ValueEntry};

/// Where one field lives, however deeply nested. `parents` descends through
/// `Nested` entries to reach the [`FormSchema`]/[`FormValues`] pair the
/// field actually belongs to; `field` is that field's index there.
///
/// `(field index, repetition index)` at each level of `parents`: a `Nested`
/// field can itself be repeated (`sh:maxCount` > 1), so reaching "the third
/// field of the second repetition of this nested field" needs both numbers,
/// not just which field.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Loc {
    pub parents: Vec<(usize, usize)>,
    pub field: usize,
}

impl Loc {
    pub fn root(field: usize) -> Self {
        Loc {
            parents: Vec::new(),
            field,
        }
    }

    pub fn child(&self, repetition: usize, field: usize) -> Self {
        let mut parents = self.parents.clone();
        parents.push((self.field, repetition));
        Loc { parents, field }
    }
}

/// Rebuilds `values` with `edit` applied to the `(schema, values)` pair
/// `loc.parents` points at, cloning only the spine on the way there (each
/// level's *other* fields are shared, not deep-copied, since `FormValues`'s
/// `entries` are cheap `Vec` clones one level at a time).
pub fn update_at(
    schema: &FormSchema,
    values: &FormValues,
    parents: &[(usize, usize)],
    edit: impl FnOnce(&mut FormValues),
) -> FormValues {
    let mut new_values = values.clone();
    match parents.split_first() {
        None => edit(&mut new_values),
        Some(((field_idx, rep_idx), rest)) => {
            let mut entries = new_values.get(*field_idx).to_vec();
            let Some(ValueEntry::Nested(inner_values)) = entries.get(*rep_idx) else {
                // The path names a slot that no longer exists (its entry was
                // removed by a concurrent edit) — nothing to update.
                return new_values;
            };
            let Some(FieldKind::Nested {
                schema: inner_schema,
            }) = schema.fields.get(*field_idx).map(|f| &f.kind)
            else {
                return new_values;
            };
            let updated_inner = update_at(inner_schema, inner_values, rest, edit);
            entries[*rep_idx] = ValueEntry::Nested(updated_inner);
            new_values.set(*field_idx, entries);
        }
    }
    new_values
}

pub fn set_leaf(
    schema: &FormSchema,
    values: &FormValues,
    loc: &Loc,
    repetition: usize,
    entry: ValueEntry,
) -> FormValues {
    update_at(schema, values, &loc.parents, move |vs| {
        let mut entries = vs.get(loc.field).to_vec();
        if repetition < entries.len() {
            entries[repetition] = entry;
        } else {
            entries.push(entry);
        }
        vs.set(loc.field, entries);
    })
}

pub fn add_entry(
    schema: &FormSchema,
    values: &FormValues,
    loc: &Loc,
    default: ValueEntry,
) -> FormValues {
    update_at(schema, values, &loc.parents, move |vs| {
        let mut entries = vs.get(loc.field).to_vec();
        entries.push(default);
        vs.set(loc.field, entries);
    })
}

pub fn remove_entry(
    schema: &FormSchema,
    values: &FormValues,
    loc: &Loc,
    repetition: usize,
) -> FormValues {
    update_at(schema, values, &loc.parents, move |vs| {
        let mut entries = vs.get(loc.field).to_vec();
        if repetition < entries.len() {
            entries.remove(repetition);
        }
        vs.set(loc.field, entries);
    })
}
