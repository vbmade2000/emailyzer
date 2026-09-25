#[allow(clippy::module_inception)]
pub mod database;
pub mod tests;
pub mod types;

pub use database::DatabaseManager;
pub use types::{DATABASE_URL, DatabaseLocation, DatabaseOperations};
