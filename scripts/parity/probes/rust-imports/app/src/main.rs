mod config;
mod store;
mod Glob;
mod glob;

use crate::config::load;
use config::Tr;
use std::fmt;
use store::Store;
use util_lib::text::shout;

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
