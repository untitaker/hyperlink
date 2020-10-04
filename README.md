# hyperlink

Very fast link checker for static sites.

* Supports traversing file-system paths only, no arbitrary URLs.

  * No support for the [`<base>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/base) tag.

  * No support for external links. It does not know how to speak HTTP.

* Does not honor `robots.txt`. A broken link is still broken for users even if
  not indexed by Google.

* Does not parse CSS files, as broken links in CSS have not been a practical
  concern for us. We are concerned about broken link in the page content, not
  the chrome around it.

* **Fast.** [docs.sentry.io](https://github.com/getsentry/sentry-docs) produces
  1.1 GB of HTML files. All [alternatives](#alternatives) we tried were too
  slow for this.

  `hyperlink` handles this amount of data in 4 seconds on a MacBook Pro 2018.

* **Pay for what you need.** By default, `hyperlink` checks for *real* 404s in
  HTML only. Anything beyond that is opt-in, such as finding broken anchors or
  future features like parsing of CSS/JS. See [Options](#options).

## Usage

[Install Rust](https://rustup.rs/), and:

```
cargo build --release
./target/release/hyperlink public/
```

## Options

* `-j/--jobs`: How many threads to spawn for parsing HTML. By default
  `hyperlink` will attempt to saturate your CPU.

* `--check-anchors`: Opt-in, check for validity of anchors on pages. Broken
  anchors are considered warnings, meaning that `hyperlink` will `exit 2` if
  there are *only* broken anchors but no hard 404s.

## Exit codes

* `exit 1`: There have been errors (hard 404s)
* `exit 2`: There have been only warnings (broken anchors)

## Alternatives

* [linkcheck](https://github.com/filiph/linkcheck) is definitely one of the
  faster linkcheckers out there, has great UX and a good set of features (more
  than hyperlink). Other than performance it worked really well for our
  usecase, and `hyperlink` takes some minor UX decisions from linkchecker here
  and there.

  We tried `linkcheck` together with
  [`http-server`](https://www.npmjs.com/package/http-server) on localhost,
  although that does not seem to be the bottleneck.

* [wummel/linkchecker](https://wummel.github.io/linkchecker/) seems to be the
  most feature rich out of all, but was a non-starter due to performance. This
  applies to other countless link checkers we tried that are not mentioned
  here.

* [Legend of Link](https://github.com/XMPPwocky/legend_of_link) is a link
  checker in Rust. `hyperlink` takes the idea of directly using a html/xml
  tokenizer from there, but does not share any code with it. We haven't been
  able to get Legend of Link running at all.

## License

Licensed under the MIT, see [`./LICENSE`](./LICENSE).
