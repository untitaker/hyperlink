# hyperlink

A command-line tool to find broken links in your static site.

* **Fast.** [docs.sentry.io](https://github.com/getsentry/sentry-docs) produces
  1.1 GB of HTML files. `hyperlink` handles this amount of data in 4 seconds on
  a MacBook Pro 2018. See [Alternatives](#alternatives) for a performance comparison.

* **Pay for what you need.** By default, `hyperlink` checks for hard 404s in
  internal links only. Anything beyond that is opt-in. See [Options](#options)
  for a list of features to enable.

* **Maps back errors to source files.** If your static site was created from
  Markdown files, `hyperlink` can try to find the original broken link by
  fuzzy-matching the content around it. See the [`--sources` option](#options).

* Supports traversing file-system paths only, no arbitrary URLs.

  * No support for the [`<base>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/base) tag.

  * [No support for external links.](https://github.com/untitaker/hyperlink/issues/5) It does not know how to speak HTTP.

  * Even if you don't have a static site, you can put hyperlink to work by
    first downloading the entire website using e.g.
    [suckit](https://github.com/Skallwar/suckit). In certain cases this is
    faster than other tools too.

* Does not honor `robots.txt`. A broken link is still broken for users even if
  not indexed by Google.

* Does not parse CSS files, as broken links in CSS have not been a practical
  concern for us. We are concerned about broken link in the page content, not
  the chrome around it.

* Only supports UTF-8 encoded HTML files.

## Installation and Usage

[Download the latest binary](https://github.com/untitaker/hyperlink/releases) and:

```bash
# Check a folder of HTML
./hyperlink public/

# Also validate anchors
./hyperlink public/ --check-anchors

# src/ is a folder of Markdown. Show original Markdown file paths in errors
./hyperlink public/ --sources src/
```

### GitHub action

```yaml
- uses: untitaker/hyperlink@0.1.26
  with:
    args: public/ --sources src/
```

### NPM

```bash
npm install -g @untitaker/hyperlink
hyperlink public/ --sources src/
```

### Docker

```bash
docker run -v $PWD:/check ghcr.io/untitaker/hyperlink:0.1.26 /check/public/ --sources /check/src/

# specific commit
docker run -v $PWD:/check ghcr.io/untitaker/hyperlink:sha-82ca78c /check/public/ --sources /check/src
```

[See all available tags](https://github.com/untitaker/hyperlink/pkgs/container/hyperlink)

### From source

```bash
cargo install hyperlink  # latest stable release
cargo install --git https://github.com/untitaker/hyperlink  # latest git SHA
```

## Options

When invoked without options, `hyperlink` only checks for 404s of internal
links. However, it can do more.

* `-j/--jobs`: How many threads to spawn for parsing HTML. By default
  `hyperlink` will attempt to saturate your CPU.

* `--check-anchors`: Opt-in, check for validity of anchors on pages. Broken
  anchors are considered warnings, meaning that `hyperlink` will `exit 2` if
  there are *only* broken anchors but no hard 404s.

* `--sources`: A folder of markdown files that were the input for the HTML
  `hyperlink` has to check. This is used to provide better error messages that
  point at the actual file to edit. `hyperlink` does very simple content-based
  matching to figure out which markdown files may have been involved in the
  creation of a HTML file.

  Why not just crawl and validate links in Markdown at this point? Answer:

  * There are countless of proprietary extensions to markdown out there for
    creating intra-page links that are generally not supported by link checking
    tools.

  * The structure of your markdown content does not necessarily match the
    structure of your HTML (i.e. what the user actually sees). With this setup,
    `hyperlink` does not have to assume anything about your build pipeline.

* `--github-actions`: Emit [GitHub actions
  errors](https://docs.github.com/en/free-pro-team@latest/actions/reference/workflow-commands-for-github-actions#setting-an-error-message),
  i.e. add error messages in-line to PR diffs. This is only useful with
  `--sources` set.

  If you are using `hyperlink` through the GitHub action this option is already
  set. It is only useful if you are downloading/building and running hyperlink
  yourself in CI.

## Exit codes

* `exit 1`: There have been errors (hard 404s)
* `exit 2`: There have been only warnings (broken anchors)

## Alternatives

*(roughly ranked by performance, determined by some unserious benchmark. this
section contains partially dated measurements and is not continuously updated
with regards to either performance or featureset)*

None of the listed alternatives have an equivalent to `hyperlink`'s `--sources`
and `--github-actions` feature.

* [lychee](https://github.com/lycheeverse/lychee), like `hyperlink`, is a great
  choice for obscenely large static sites. Additionally it can check
  external/outbound links. An invocation of `lychee --offline public/` is more or
  less equivalent to `hyperlink public/`.

* [liche](https://github.com/raviqqe/liche) seems to be fairly fast, but is
  unmaintained.

* [htmltest](https://github.com/wjdp/htmltest) seems to be fairly fast as well,
  and is more of a general-purpose HTML linting tool.

* [muffet](https://github.com/raviqqe/muffet) seems to have similar performance
  as `htmltest`. We tested `muffet` with
  [`http-server`](https://www.npmjs.com/package/http-server) and webfsd without
  noticing a change in timings.

* [linkcheck](https://github.com/filiph/linkcheck) is faster than `linkchecker`
  but still quite slow on large sites.

  We tried `linkcheck` together with
  [`http-server`](https://www.npmjs.com/package/http-server) on localhost,
  although that does not seem to be the bottleneck at all.

* [wummel/linkchecker](https://wummel.github.io/linkchecker/) seems to be the
  fairly feature-rich, but was a non-starter due to performance. This applies
  to other countless link checkers we tried that are not mentioned here.

## Testimonials

> We use Hyperlink to check for dead links on
> [Graphviz's static-site user documentation](https://graphviz.org/), because:
> 
> * Hyperlink is *blazingly* fast, checking 700 HTML pages in 220ms (default) and
>   850ms (with `--check-anchors`).
> * Hyperlink's single-binary release, with no library dependencies,
>   was trivial to integrate into our [continuous integration tests](https://gitlab.com/graphviz/graphviz.gitlab.io/-/blob/5dcfa637b7df17e3a1b821f3d7e9de8f5f82544b/.gitlab-ci.yml#L27).
> * High coverage: Hyperlink immediately spotted over a thousand broken page
>   links within both `<a>` tags and HTML redirects, and a further 62 broken
>   anchor-links with `--check-anchors`.
> * Hyperlink's design decision to crawl only static files (avoiding HTTP),
>   avoids test flakiness from network requests, allowing me to confidently
>   block merging if Hyperlink reports an error.
>
> In conclusion, Hyperlink fills the "static site continuous testing" niche
> really nicely.

-- Mark Hansen, Graphviz documentation maintainer

## License

Licensed under the MIT, see [`./LICENSE`](./LICENSE).
