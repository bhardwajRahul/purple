// Runtime support for the binary entry. Hosts the helpers that previously
// lived in src/main.rs so library code can reach them without the bin crate
// having to re-export them across the lib/bin boundary.
pub mod env;
pub mod helpers;
pub mod launcher;
pub mod layout;
