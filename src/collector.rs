use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use crate::html::{Href, Link, UsedLink};
use crate::paragraph::Paragraph;

pub trait LinkCollector<'a>: Send {
    fn new() -> Self;
    fn ingest(&mut self, link: Link<'a, Paragraph>);
    fn merge(&mut self, other: Self);
}

/// Collects only used links for match-all-paragraphs command. Discards defined links.
#[derive(Default)]
pub struct UsedLinkCollector<'a> {
    pub used_links: BTreeSet<UsedLink<'a, Paragraph>>,
}

impl<'a> LinkCollector<'a> for UsedLinkCollector<'a> {
    fn new() -> Self {
        Default::default()
    }

    fn ingest(&mut self, link: Link<'a, Paragraph>) {
        if let Link::Uses(used_link) = link {
            self.used_links.insert(used_link);
        }
    }

    fn merge(&mut self, other: Self) {
        self.used_links.extend(other.used_links);
    }
}

#[derive(Debug, Eq, PartialEq)]
enum LinkState<'a> {
    /// We have observed a DefinedLink for this href
    Defined,
    /// We have not *yet* observed a DefinedLink and therefore need to keep track of all link
    /// usages for potential error reporting.
    Undefined(Vec<UsedLink<'a, Paragraph>>),
}

impl<'a> LinkState<'a> {
    fn add_usage(&mut self, link: UsedLink<'a, Paragraph>) {
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
#[derive(Default)]
pub struct BrokenLinkCollector<'a> {
    used_links: BTreeMap<Href<'a>, LinkState<'a>>,
}

impl<'a> LinkCollector<'a> for BrokenLinkCollector<'a> {
    fn new() -> Self {
        Default::default()
    }

    fn ingest(&mut self, link: Link<'a, Paragraph>) {
        match link {
            Link::Uses(used_link) => {
                self.used_links
                    .entry(used_link.href)
                    .or_insert_with(|| LinkState::Undefined(Vec::new()))
                    .add_usage(used_link);
            }
            Link::Defines(defined_link) => {
                self.used_links
                    .insert(defined_link.href, LinkState::Defined);
            }
        }
    }

    fn merge(&mut self, other: Self) {
        for (href, state) in other.used_links {
            match self.used_links.entry(href) {
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
pub struct BrokenLink<'a, P> {
    pub hard_404: bool,
    pub used_link: UsedLink<'a, P>,
}

impl<'a> BrokenLinkCollector<'a> {
    pub fn get_broken_links(
        &self,
        check_anchors: bool,
    ) -> impl Iterator<Item = BrokenLink<'a, Paragraph>> {
        let mut broken_links = BTreeSet::new();

        for (&href, state) in &self.used_links {
            if let LinkState::Undefined(links) = state {
                let hard_404 = if check_anchors {
                    self.used_links.get(&href.without_anchor()) != Some(&LinkState::Defined)
                } else {
                    true
                };

                for &used_link in links {
                    broken_links.insert(BrokenLink {
                        used_link,
                        hard_404,
                    });
                }
            }
        }

        broken_links.into_iter()
    }

    pub fn used_links_count(&self) -> usize {
        self.used_links.len()
    }
}
