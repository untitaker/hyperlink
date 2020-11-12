use patricia_tree::PatriciaMap;

use crate::html::{Href, Link, UsedLink};

impl AsRef<[u8]> for Href {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

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

#[derive(Debug)]
enum LinkState<P> {
    /// We have observed a DefinedLink for this href
    Defined,
    /// We have not *yet* observed a DefinedLink and therefore need to keep track of all link
    /// usages for potential error reporting.
    Undefined(PatriciaMap<Option<P>>),  // (path) -> paragraph
}

impl<P: Copy> LinkState<P> {
    fn add_usage(&mut self, link: &UsedLink<P>) {
        if let LinkState::Undefined(ref mut links) = self {
            links.insert(
                link.path.to_str().expect("Invalid UTF-8 in path"),
                link.paragraph.as_ref().map(|&x| x)
            );
        }
    }

    fn update(&mut self, other: Self) {
        match self {
            LinkState::Defined => (),
            LinkState::Undefined(links) => {
                match other {
                    LinkState::Defined => *self = LinkState::Defined,
                    LinkState::Undefined(links2) => links.extend(links2.into_iter()),
                }
            }
        }
    }
}

/// Link collector used for actual link checking. Keeps track of broken links only.
pub struct BrokenLinkCollector<P> {
    links: PatriciaMap<LinkState<P>>,
    used_link_count: usize,
}

impl<P: Send + Copy> LinkCollector<P> for BrokenLinkCollector<P> {
    fn new() -> Self {
        BrokenLinkCollector {
            links: PatriciaMap::new(),
            used_link_count: 0,
        }
    }

    fn ingest(&mut self, link: Link<P>) {
        match link {
            Link::Uses(used_link) => {
                self.used_link_count += 1;
                if let Some(state) = self.links.get_mut(&used_link.href) {
                    state.add_usage(&used_link);
                } else {
                    let mut state = LinkState::Undefined(PatriciaMap::new());
                    state.add_usage(&used_link);
                    self.links.insert(used_link.href, state);
                }
            }
            Link::Defines(defined_link) => {
                self.links.insert(defined_link.href, LinkState::Defined);
            }
        }
    }

    fn merge(&mut self, other: Self) {
        self.used_link_count += other.used_link_count;

        for (href, other_state) in other.links {
            if let Some(state) = self.links.get_mut(&href) {
                state.update(other_state);
            } else {
                self.links.insert(href, other_state);
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

        for (href, state) in self.links.iter() {
            if let LinkState::Undefined(links) = state {
                let href = unsafe { Href(String::from_utf8_unchecked(href)) };
                let hard_404 = if check_anchors {
                    !matches!(self.links.get(&href.without_anchor()), Some(&LinkState::Defined))
                } else {
                    true
                };

                for (path, &paragraph) in links.iter() {
                    broken_links.push(BrokenLink {
                        used_link: UsedLink {
                            href: href.clone(),
                            path: unsafe { String::from_utf8_unchecked(path).into() },
                            paragraph,
                        },
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
