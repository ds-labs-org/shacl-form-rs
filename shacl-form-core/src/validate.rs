//! Checks one [`ValueEntry`] against its [`Field`]'s own derived SHACL
//! constraints (`sh:pattern`/`sh:minLength`/`sh:maxLength`/numeric bounds)
//! as executable Rust, not just the HTML attributes `shacl-form-yew`'s
//! built-in controls encode them as. Native browser constraint validation
//! only ever covers a real `<input>`/`<select>` this crate itself
//! rendered; a host-supplied custom control has no such attribute to lean
//! on, so it needs this instead — see `shacl-form-yew`'s own
//! `FieldOverride`, which runs this alongside its own extra validator.
use crate::model::{Field, FieldKind};
use crate::values::ValueEntry;

/// `Ok(())` when `entry` satisfies every constraint `field.kind` carries;
/// otherwise a human-readable reason. An empty entry (the lexical form an
/// untouched, not-yet-typed-into control holds — see `default_entry`) is
/// never checked: `FormValues::to_turtle` already treats it as "not really
/// a value yet" and skips serialising it, so failing it here would reject
/// a blank required field before the user has had a chance to fill it in;
/// cardinality (`sh:minCount`) is a field-level, not per-entry, concern
/// this function does not attempt.
pub fn check_constraints(field: &Field, entry: &ValueEntry) -> Result<(), String> {
    match &field.kind {
        FieldKind::Text {
            patterns,
            min_length,
            max_length,
        }
        | FieldKind::Iri {
            patterns,
            min_length,
            max_length,
        } => {
            let lexical = entry_lexical(entry);
            if lexical.is_empty() {
                return Ok(());
            }
            check_length(&lexical, *min_length, *max_length)?;
            check_patterns(&lexical, patterns)?;
        }
        FieldKind::Number {
            integer_only,
            min,
            max,
        } => {
            if let ValueEntry::Literal(lit) = entry {
                if lit.value().is_empty() {
                    return Ok(());
                }
                let value: f64 = lit
                    .value()
                    .parse()
                    .map_err(|_| format!("\"{}\" is not a number", lit.value()))?;
                if *integer_only && value.fract() != 0.0 {
                    return Err(format!("\"{}\" is not a whole number", lit.value()));
                }
                if let Some(min) = min
                    && value < *min
                {
                    return Err(format!("must be at least {min}"));
                }
                if let Some(max) = max
                    && value > *max
                {
                    return Err(format!("must be at most {max}"));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn entry_lexical(entry: &ValueEntry) -> String {
    match entry {
        ValueEntry::Literal(l) => l.value().to_string(),
        ValueEntry::Node(n) => n.as_str().to_string(),
        ValueEntry::Nested { .. } => String::new(),
    }
}

fn check_length(lexical: &str, min: Option<u32>, max: Option<u32>) -> Result<(), String> {
    // Unicode scalar values (code points), matching SHACL's own definition
    // of string length — not the UTF-16 code units the HTML `minlength`/
    // `maxlength` attributes count (see the README's own note on this),
    // since this function isn't bound by what an `<input>` can express.
    let len = lexical.chars().count() as u32;
    if let Some(min) = min
        && len < min
    {
        return Err(format!("must be at least {min} characters (got {len})"));
    }
    if let Some(max) = max
        && len > max
    {
        return Err(format!("must be at most {max} characters (got {len})"));
    }
    Ok(())
}

/// SHACL's `sh:pattern` is an unanchored (substring) match — exactly what
/// `Regex::is_match` already does, no anchoring trick needed (unlike
/// `shacl-form-yew`'s `controls.rs::combined_pattern`, which has to fight
/// HTML's always-anchored `pattern` attribute with a lookahead `regex`, a
/// linear-time engine with no backtracking, cannot even compile). A
/// pattern that fails to compile here is treated as unverifiable — a
/// malformed `sh:pattern` is a shape-authoring problem, not this
/// particular value's fault, so it does not block submission.
fn check_patterns(lexical: &str, patterns: &[String]) -> Result<(), String> {
    for p in patterns {
        if let Ok(re) = regex::Regex::new(p)
            && !re.is_match(lexical)
        {
            return Err(format!("must match the pattern {p}"));
        }
    }
    Ok(())
}
