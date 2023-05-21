# Overview over how hyperlink operates (or: why is it so fast?)

In this document we'll explore how
[hyperlink](https://github.com/untitaker/hyperlink), a link checker for static
sites, behaves differently from a general-purpose link checker for checking
static sites, and why that makes it faster (in theory and often in practice)

Typically you'd think of a website and its links as a graph to traverse, where
the nodes are HTML documents and the edges are links between the documents.
Therefore, if you point a general-purpose link checker like
[muffet](https://github.com/raviqqe/muffet) or
[lychee](https://github.com/lycheeverse/lychee) at the root of your website,
they typically will:

* Put that root URL into a queue of some sort
* For each element in the queue, do:
  1. Fetch the underlying document, and report errors like 404 along the way.
  2. Extract all links from the document, put them back into the queue and repeat.

There's two performance problems with this approach that hyperlink avoids:

1. Treating websites as graphs to discover can impact parallelism. The few
   initial milliseconds will be spent parsing the root document (presumably on
   a single thread), and if you're unlucky, you may not actually discover
   enough links to hit concurrency limits and fully make use of available
   resources.

   Most link checkers are smart enough to initialize the queue with all files
   on the file system in cases where a folder of files should be checked, but
   it's completely unavoidable when traversing arbitrary URLs over the
   internet. Hyperlink has it relatively easy by only supporting static sites
   on the local filesystem and not supporting any external links.

2. Having a global queue can also impact parallelism. If checking external
   links, your work is purely I/O bound, but if you're checking a folder of
   HTML for broken internal links, it may not be as much. When it comes to
   that, things like lock contention start to matter, and regardless of how
   your queue works, you have shared global state between your threads that
   impacts parallelism.

   At the same time, if you're traversing a live website over HTTP, you
   probably _do_ want a (strongly?) consistent, single source of truth for
   which links have been visited already and which still need to be traversed.
   Because the alternative is to check some links twice.

Hyperlink neither has a global queue, nor does it regard your website as a
graph of links. First, it defines datatypes like the following:

```rust
enum LinkState {
    Defined,
    Undefined(LinkUsers),
}

type LinkCollector = HashMap<String, LinkState>;
type LinkUsers = Vec<String>;
```

Then it runs the following procedure:

1. Enumerate all files on the filesystem.

2. For each file, create one or more `LinkCollector`s:

    * For a file `/index.html`, hyperlink will create a `LinkCollector` such as:

      ```json
      {
          "/index.html": LinkState::Defined
      }
      ```

    * For a parsed link `/about.html` inside of that file, it will create:

      ```json
      {
          "/about.html": LinkState::Undefined(["/index.html"])
      }
      ```

    * For a file `/about.html`, it will create:

      ```json
      {
          "/about.html": LinkState::Defined
      }
      ```

    * For a parsed link `/404.html` inside of that file, it will create:

      ```json
      {
          "/404.html": LinkState::Undefined(["/about.html"])
      }
      ```

3. Merge all `LinkCollector`s into one, just as one would merge hashmaps
   together. For conflicting keys, these rules apply:

   * `Defined` wins over `Undefined`
   * Two conflicting `Undefined` values are concatenated together.

   In the above example, we'd be left with this:

   ```json
   {
       "/index.html": LinkState::Defined,
       "/about.html": LinkState::Defined,
       "/404.html": LinkState::Undefined(["/about.html"])
   }
   ```

4. At the end, one `LinkCollector` remains. After filtering out all the
   `Defined` entries, hyperlink now knows that the link to `/404.html` inside
   of the file `/about.html` is broken.

Basically, we did a [map-reduce](https://en.wikipedia.org/wiki/MapReduce) here.
The great part about this is that there's no shared state across threads in
Step 2, and Step 3 minimizes the amount of shared state because merging two
`LinkCollector`s can happen independently of other merging operations.

## Implementation

* Hyperlink collects the list of files using the `jwalk` crate initially (Step
  1), then uses `rayon` for the "real" parallelization.

* Hyperlink does not actually create multiple `LinkCollector` instances (i.e.
  multiple hashmaps per file). Instead there's one `LinkCollector` per worker
  thread that gets continuously updated with new entries.

* ...a few micro-optimizations. Most notably:

  * attempts to avoid as many per-link allocations as possible by using the
    `bumpalo` crate.

    The excessive use of `bumpalo` forces using lot of explicit lifetimes and
    makes the code less maintainable, but luckily we're not building a library
    with a stable, public API here!

  * attempts to lower memory usage by using a radix tree instead of a hashmap
    inside of `LinkCollector`. Since there's a lot of common path prefixes in
    URLs, this does lower memory usage quite a bit. Memory usage is usually
    insignificant, however, so this only matters for some pathological cases.

  * [custom HTML5 parser](https://github.com/untitaker/html5gum) with
    customizable allocations, because html5ever was too slow and quick-xml too
    strict.
