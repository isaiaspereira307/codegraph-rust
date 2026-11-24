use super::config;
use crate::config::Tr;

pub struct Disk;

impl Disk {
    pub fn new() -> Disk {
        Disk
    }

    pub fn open() {
        config::load();
    }
}

impl Tr for Disk {
    fn run(&self) {}
}
