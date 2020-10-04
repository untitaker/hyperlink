# hyperlink

Very fast link checker for CI.

* Supports traversing file-system paths only, no arbitrary URLs.

  * No support for the [`<base>`](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/base) tag.

  * No support for external links. It does not know how to speak HTTP.

* Does not parse/validate anchors *yet*.

* Does not honor `robots.txt`. A broken link is still broken for users even if
  not indexed by Google.

* Does not parse CSS files, as broken links in CSS have not been a practical
  concern for us. We are concerned about broken link in the page content, not
  the chrome around it.

* **Fast.** On [docs.sentry.io](https://github.com/getsentry/sentry-docs),
  [linkcheck](https://github.com/filiph/linkcheck) takes 10 minutes on a
  MacBook Pro 2018, this one takes 4 seconds. We tried `linkcheck` together
  with [`http-server`](https://www.npmjs.com/package/http-server) on localhost,
  although that does not seem to be the bottleneck.

  In fairness, `hyperlink` does less, but in our case we need less. If it ever
  does more, it will be possible to disable that to restore performance.

## Usage

[Install Rust](https://rustup.rs/), and:

```
cargo build --release
./target/release/hyperlink public/
```

## Options

* `-j/--jobs`: How many threads to spawn for parsing HTML. By default
  `hyperlink` will attempt to saturate your CPU.

* `--check-unreachable`: Also check for *unreachable* HTML pages, i.e. pages
  that have not been linked to from anywhere. This is disabled by default.

  Unreachable pages are considered warnings as they don't actually disrupt the
  usability of the page.

## Exit codes

* `exit 1`: There have been errors.
* `exit 2`: There have been only warnings.

## License

Licensed under the MIT, see [`./LICENSE`](./LICENSE).
