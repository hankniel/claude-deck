//! Canvas: screens an agent writes during a session (diagrams, lineage, ERDs, review findings,
//! row counts, documents), an objectives board, and the hooks that connect them to the agent.
//! Pure data and layout; the desktop apps draw it.

pub mod dbml;
pub mod flow;
pub mod git;
pub mod jira;
pub mod hooks;
pub mod layout;
pub mod lineage;
pub mod objectives;
pub mod plan;
pub mod scene;
pub mod screens;
pub mod sequence;
pub mod store;
