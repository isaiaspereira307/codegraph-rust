pub mod disk;

use self::disk::Disk;

pub struct Store;

impl Store {
    pub fn new() -> Store {
        Disk::open();
        Store
    }
}
