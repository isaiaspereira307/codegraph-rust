mod config;
mod store;
mod Glob;
mod glob;
mod repo;

use crate::config::load;
use config::Tr;
use std::fmt;
use store::Store;
use util_lib::text::shout;
use crate::repo::Repo;
use store::alt::alt_call;
use store::Ship;

fn main() {
    load();
    config::load();
    shout();
    store::disk::Disk::open();
    let _v: Vec<u8> = Vec::new();
    Store::new();
    Glob::upper();
    glob::lower();
}

fn show(_f: &fmt::Formatter) {}

pub use config::Tr as Exported;

struct Local;

impl Exported for Local {
    fn run(&self) {}
}

fn typed_receiver() {
    let repo = Repo::new();
    repo.save();
}

fn wildcard_use() {
    alt_call();
    Ship::run(&Repo);
}
fn super_above_root() {
    super::config::load();
}
