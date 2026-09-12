//! proxplore providers — every source, each identified by its exact id.
//!
//! The 16 cron-refreshed GitHub raw repos share one data table
//! (github_feeds.rs, composed on the GithubFeed base); the 14 sources with
//! real logic — JSON APIs, pagination, bespoke HTML — own one module each.
//! Everything is wired into `all()`; the filesystem guard in main.rs catches
//! a module created but never registered.

use std::sync::Arc;

use crate::model::Provider;

mod github_feed;
mod github_feeds;

// --- API / JSON feeds -----------------------------------------------------
mod advancedname;
mod databay;
mod geonode;
mod m1noa_proxypool;
mod proxifly;
mod proxygenerator1;
mod proxylister;
mod proxyscrape;
mod pubproxy;
mod spys;

// --- HTML list pages (bespoke extraction) ---------------------------------
mod e89ip;
mod free_proxy_list;
mod ip3366;
mod proxydb;

pub fn all() -> Vec<Arc<dyn Provider>> {
    let mut v = github_feeds::all();
    v.extend([
        advancedname::new(),
        databay::new(),
        e89ip::new(),
        free_proxy_list::new(),
        geonode::new(),
        ip3366::new(),
        m1noa_proxypool::new(),
        proxifly::new(),
        proxylister::new(),
        proxyscrape::new(),
        pubproxy::new(),
        proxydb::new(),
        proxygenerator1::new(),
        spys::new(),
    ]);
    v
}
