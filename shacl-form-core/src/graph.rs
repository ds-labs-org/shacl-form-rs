//! Turtle in, `oxrdf::Graph` out — the one place this crate touches `oxttl`
//! directly, so a future second syntax (JSON-LD, RDF/XML) is one function to
//! add here, not a change scattered through [`crate::schema`].
use crate::error::ShapesError;
use oxrdf::Graph;
use oxttl::TurtleParser;

pub fn parse_turtle(turtle: &str) -> Result<Graph, ShapesError> {
    let mut graph = Graph::new();
    for triple in TurtleParser::new().for_slice(turtle.as_bytes()) {
        graph.insert(&triple?);
    }
    Ok(graph)
}

/// Same parse, a different error variant — an instance document failing to
/// parse is a different problem for a caller than the *shapes* graph
/// failing to (one is "this shape file is broken", the other "the form you
/// filled in came back malformed"), so [`crate::error::ShapesError`] keeps
/// them apart rather than reusing one variant for both call sites.
pub fn parse_instance_turtle(turtle: &str) -> Result<Graph, ShapesError> {
    let mut graph = Graph::new();
    for triple in TurtleParser::new().for_slice(turtle.as_bytes()) {
        graph.insert(&triple.map_err(ShapesError::InstanceTurtle)?);
    }
    Ok(graph)
}
