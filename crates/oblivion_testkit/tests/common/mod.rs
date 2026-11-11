//! IDA: N/A what the Phase 12 test files share: the row book, object images and the load harness. Each test binary uses part of this.
#![allow(
    dead_code,
    reason = "each test binary uses only part of the shared helpers"
)]

pub mod act_rows;
pub mod image;
pub mod images;
pub mod images_ai_env;
pub mod images_dialogue;
pub mod images_items;
pub mod images_magic_actors;
pub mod images_world;
pub mod load;
pub mod rows;
