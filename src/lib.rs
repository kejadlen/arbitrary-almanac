#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

pub mod bundle;
pub mod catalog;
mod error;
pub mod host;
mod renovate;

pub use catalog::Catalog;
pub use error::Error;
