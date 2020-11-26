#[macro_use]
extern crate diesel;

#[macro_use]
extern crate serde;

#[macro_use]
extern crate rocket;

#[macro_use]
extern crate tracing;

pub mod schema;
pub mod db;
pub mod model;
pub mod pass;
