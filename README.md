# hyperlink

A very fast and simple link checker to be run in CI. Was created because
[linkcheck](https://github.com/filiph/linkcheck) was too slow for us, although
otherwise it worked well.

* Supports file-system paths only.
* No support for external links.
* No support for anchors *for now*. If anchors support is added, it will be
  opt-out for those wo want to preserve performance.
* Only checks links that are actually likely to break in your average static
  site. That means no odd crawling of included CSS and JavaScript.
* Abililty to actually max out your CPU.

## Usage

[Install Rust](https://rustup.rs/), and:

```
cargo build --release
./target/release/hyperlink public/
```

## License

Licensed under the MIT, see [`./LICENSE`](./LICENSE).
