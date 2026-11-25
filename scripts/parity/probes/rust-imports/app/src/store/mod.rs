pub mod disk;
#[path = "alt.rs"]
pub mod alt;
pub use super::config::Tr as Ship;

use self::disk::Disk;

pub struct Store;

impl Store {
    pub fn new() -> Store {
        Disk::open();
        Store
    }
}
