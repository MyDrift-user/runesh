#![deny(unsafe_code)]
pub mod diff;
pub mod error;
pub mod eval;
pub mod hujson;
pub mod model;

pub use diff::AclDiff;
pub use error::AclError;
pub use eval::{AclEvalResult, AclEvaluator, EvalContext, GroupResolver};
pub use model::{
    AclAction, AclPolicy, AclRule, AclTarget, DstTarget, PortRange, PortSet, parse_dst,
    parse_target,
};
