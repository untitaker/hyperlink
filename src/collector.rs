use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use crate::html::{Href, Link, UsedLink};

pub trait LinkCollector<P: Send>: Send {
    fn new() -> Self;
    fn ingest(&mut self, link: Link<P>);
    fn merge(&mut self, other: Self);
}

/// Collects only used links for match-all-paragraphs command. Discards defined links.
pub struct UsedLinkCollector<P> {
    pub used_links: Vec<UsedLink<P>>,
}

impl<P: Send> LinkCollector<P> for UsedLinkCollector<P> {
    fn new() -> Self {
        UsedLinkCollector {
            used_links: Vec::new(),
        }
    }

    fn ingest(&mut self, link: Link<P>) {
        if let Link::Uses(used_link) = link {
            self.used_links.push(used_link);
        }
    }

    fn merge(&mut self, other: Self) {
        self.used_links.extend(other.used_links);
    }
}

#[derive(Debug, Eq, PartialEq)]
enum LinkState<P> {
    /// We have observed a DefinedLink for this href
    Defined,
    /// We have not *yet* observed a DefinedLink and therefore need to keep track of all link
    /// usages for potential error reporting.
    Undefined(Vec<UsedLink<P>>),
}

impl<P> LinkState<P> {
    fn add_usage(&mut self, link: UsedLink<P>) {
        if let LinkState::Undefined(ref mut links) = self {
            links.push(link);
        }
    }

    fn update(&mut self, mut other: Self) {
        match (self, &mut other) {
            (LinkState::Defined, _) => (),
            (slf, LinkState::Defined) => *slf = LinkState::Defined,
            (LinkState::Undefined(links), LinkState::Undefined(links2)) => {
                links.extend(links2.drain(..))
            }
        }
    }
}

/// Link collector used for actual link checking. Keeps track of broken links only.
pub struct BrokenLinkCollector<P> {
    links: BTreeMap<Href, LinkState<P>>,
    used_link_count: usize,
}

impl<P: Send> LinkCollector<P> for BrokenLinkCollector<P> {
    fn new() -> Self {
        BrokenLinkCollector {
            links: BTreeMap::new(),
            used_link_count: 0,
        }
    }

    fn ingest(&mut self, link: Link<P>) {
        match link {
            Link::Uses(used_link) => {
                self.used_link_count += 1;
                self.links
                    .entry(used_link.href.clone())
                    .or_insert_with(|| LinkState::Undefined(Vec::new()))
                    .add_usage(used_link);
            }
            Link::Defines(defined_link) => {
                self.links.insert(defined_link.href, LinkState::Defined);
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.used_link_count += other.used_link_count;

        for (href, state) in other.links {
            match self.links.entry(href) {
                Entry::Occupied(mut entry) => {
                    entry.get_mut().update(state);
                }
                Entry::Vacant(entry) => {
                    entry.insert(state);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct BrokenLink<P> {
    pub hard_404: bool,
    pub used_link: UsedLink<P>,
}

impl<P: Copy + PartialEq> BrokenLinkCollector<P> {
    pub fn get_broken_links(&self, check_anchors: bool) -> impl Iterator<Item = BrokenLink<P>> {
        let mut broken_links = Vec::new();

        for (href, state) in &self.links {
            if let LinkState::Undefined(links) = state {
                let hard_404 = if check_anchors {
                    self.links.get(&href.without_anchor()) != Some(&LinkState::Defined)
                } else {
                    true
                };

                for used_link in links {
                    broken_links.push(BrokenLink {
                        used_link: used_link.clone(),
                        hard_404,
                    });
                }
            }
        }

        broken_links.into_iter()
    }

    pub fn used_links_count(&self) -> usize {
        self.used_link_count
    }
}
