//! The `sh:` (SHACL Core, <http://www.w3.org/ns/shacl#>) terms this crate
//! reads, named the same way `oxrdf::vocab::xsd` names its own.
//!
//! Not every SHACL Core term lives here — only the ones [`crate::schema`]
//! actually branches on. A predicate this crate doesn't recognise is not
//! silently invisible: [`crate::schema::walk`] records it in
//! [`crate::model::FormSchema::unsupported`] rather than pretending the shape
//! said less than it did.
use oxrdf::NamedNodeRef;

macro_rules! sh_terms {
    ($($name:ident => $local:literal),* $(,)?) => {
        $(pub const $name: NamedNodeRef<'static> =
            NamedNodeRef::new_unchecked(concat!("http://www.w3.org/ns/shacl#", $local));)*
    };
}

sh_terms! {
    NODE_SHAPE => "NodeShape",
    PROPERTY => "property",
    PATH => "path",
    TARGET_CLASS => "targetClass",
    TARGET_NODE => "targetNode",
    NAME => "name",
    DESCRIPTION => "description",
    ORDER => "order",
    DATATYPE => "datatype",
    CLASS => "class",
    NODE => "node",
    NODE_KIND => "nodeKind",
    MIN_COUNT => "minCount",
    MAX_COUNT => "maxCount",
    MIN_LENGTH => "minLength",
    MAX_LENGTH => "maxLength",
    PATTERN => "pattern",
    FLAGS => "flags",
    MIN_INCLUSIVE => "minInclusive",
    MAX_INCLUSIVE => "maxInclusive",
    MIN_EXCLUSIVE => "minExclusive",
    MAX_EXCLUSIVE => "maxExclusive",
    IN => "in",
    HAS_VALUE => "hasValue",
    DEFAULT_VALUE => "defaultValue",
    OR => "or",
    AND => "and",
    NOT => "not",
    XONE => "xone",
    CLOSED => "closed",
    IGNORED_PROPERTIES => "ignoredProperties",
    IRI => "IRI",
    BLANK_NODE => "BlankNode",
    LITERAL => "Literal",
    IRI_OR_LITERAL => "IRIOrLiteral",
    BLANK_NODE_OR_IRI => "BlankNodeOrIRI",
    BLANK_NODE_OR_LITERAL => "BlankNodeOrLiteral",
    INVERSE_PATH => "inversePath",
    ALTERNATIVE_PATH => "alternativePath",
    ZERO_OR_MORE_PATH => "zeroOrMorePath",
    ONE_OR_MORE_PATH => "oneOrMorePath",
    ZERO_OR_ONE_PATH => "zeroOrOnePath",
}
