use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShapesError {
    #[error("could not parse the shapes graph as Turtle: {0}")]
    Turtle(#[from] oxttl::TurtleSyntaxError),
    #[error("could not parse the instance graph as Turtle: {0}")]
    InstanceTurtle(oxttl::TurtleSyntaxError),
    #[error("<{0}> is not `a sh:NodeShape` in this shapes graph")]
    NotANodeShape(String),
    #[error("no `sh:NodeShape` in this shapes graph has `sh:targetClass <{0}>`")]
    NoShapeForClass(String),
    #[error(
        "shape <{0}> refers to itself through `sh:node`/`sh:class` with no base case — refusing rather than recursing forever"
    )]
    CyclicShape(String),
}
